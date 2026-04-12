use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ProviderRunOperationLanes, ProviderRunState};
use crate::pty::PtyProcessState;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptStatus, PromptSubmissionOutcome,
};
use crate::transport::flow_control;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse, RelayPromptAttachment};
use arroba_relay::protocol::ClientTarget;
use base64::Engine;
use std::fs;
use std::time::Duration;

pub(crate) struct KernelPromptSubmission {
    pub(crate) outcome: PromptSubmissionOutcome,
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) dispatch: Option<KernelPromptDispatch>,
}

pub(crate) struct KernelPromptDispatch {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<PromptAttachment>,
}

pub(crate) struct KernelPromptCancellation {
    pub(crate) cancellation: PromptCancellation,
    pub(crate) dispatch: Option<KernelPromptAbortDispatch>,
}

pub(crate) struct KernelPromptAbortDispatch {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
}

impl DaemonApp {
    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.ensure_attachment_in_session(session_id, attachment_id)?;
        let session_before = self.sessions.get_session(session_id)?;

        let target_agent_id = target_agent_id
            .or_else(|| session_before.focused_agent_id())
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "no focused agent".to_string(),
            })?
            .to_string();
        let target_agent = self.agents.get_agent(&target_agent_id)?;
        let remote_execution = target_agent.remote_execution().cloned();
        let queued_while_active = session_before
            .active_prompt_for_agent(&target_agent_id)
            .is_some();
        let provider_run_id = if remote_execution.is_some() {
            None
        } else if queued_while_active {
            self.providers
                .get_run_for_agent(session_id, &target_agent_id)
                .map(|run| run.id().to_string())
        } else {
            Some(self.ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)?)
        };
        let provider_run_is_starting = provider_run_id
            .as_deref()
            .and_then(|provider_run_id| self.providers.get_run(provider_run_id).ok())
            .is_some_and(|run| run.state() == ProviderRunState::Starting);

        self.append_user_prompt_history(
            session_id,
            attachment_id,
            &target_agent_id,
            prompt,
            &attachments,
        );

        let (_session, outcome) = if provider_run_is_starting {
            self.sessions.queue_prompt(
                session_id,
                attachment_id,
                &target_agent_id,
                prompt,
                attachments.clone(),
            )?
        } else {
            self.sessions.submit_prompt(
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
                        self.block_on_relay_future(send_peer_request_via_temporary_connection(
                            &self.config,
                            ClientTarget {
                                daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                                daemon_alias: None,
                            },
                            RelayPeerRequest::SubmitLeasedPrompt {
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                                prompt: prompt.prompt().to_string(),
                                attachments: self
                                    .serialize_remote_prompt_attachments(prompt.attachments())?,
                                workflow_context: None,
                            },
                        ));
                    let remote_provider_run_id = match response {
                        Ok(RelayPeerResponse::LeasedPromptSubmitted {
                            provider_run_id, ..
                        }) => provider_run_id,
                        Ok(other) => {
                            let _ = self
                                .sessions
                                .cancel_active_prompt(session_id, &target_agent_id);
                            return Err(DaemonError::LocalTransport {
                                operation: "submit remote prompt",
                                message: format!("unexpected remote prompt response: {other:?}"),
                            });
                        }
                        Err(error) => {
                            let _ = self
                                .sessions
                                .cancel_active_prompt(session_id, &target_agent_id);
                            return Err(error);
                        }
                    };
                    self.echo_prompt_to_other_attachments(
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
                    let _ = self
                        .sessions
                        .cancel_active_prompt(session_id, &target_agent_id);
                    flow_control::clear_prompt_activity(self, provider_run_id);
                    return Err(error);
                }
                flow_control::note_prompt_started(self, provider_run_id);
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

        Ok(outcome)
    }

    pub(crate) fn prepare_provider_prompt_dispatch(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let _ = self.reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }
        Ok(provider_run)
    }

    pub(crate) fn finish_kernel_prompt_dispatch(
        &mut self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        result: Result<(), DaemonError>,
    ) -> Result<(), DaemonError> {
        if let Err(error) = result {
            let _ = self.sessions.cancel_active_prompt(&session_id, &agent_id);
            flow_control::clear_prompt_activity(self, &provider_run_id);
            self.record_notice(
                &session_id,
                Some(&provider_run_id),
                self.attachments.list_session_attachment_ids(&session_id),
                format!("Prompt dispatch failed after acknowledgement: {error}"),
            );
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn enqueue_kernel_prompt_dispatch(
        &mut self,
        dispatch: &KernelPromptDispatch,
    ) -> Result<(), DaemonError> {
        let provider_run =
            self.prepare_provider_prompt_dispatch(&dispatch.session_id, &dispatch.provider_run_id)?;
        self.providers.enqueue_structured_prompt_submit(
            dispatch.session_id.clone(),
            dispatch.provider_run_id.clone(),
            dispatch.agent_id.clone(),
            &provider_run,
            &dispatch.prompt,
            &dispatch.attachments,
        )
    }

    pub(crate) fn fail_kernel_prompt_dispatch(
        &mut self,
        dispatch: KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        let _ = self
            .sessions
            .cancel_active_prompt(&dispatch.session_id, &dispatch.agent_id);
        flow_control::clear_prompt_activity(self, &dispatch.provider_run_id);
        self.record_notice(
            &dispatch.session_id,
            Some(&dispatch.provider_run_id),
            self.attachments
                .list_session_attachment_ids(&dispatch.session_id),
            format!("Prompt dispatch failed after acknowledgement: {error}"),
        );
        Err(error)
    }

    pub(crate) fn spawn_kernel_prompt_dispatch_operation(
        app: std::sync::Arc<tokio::sync::Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        dispatch: KernelPromptDispatch,
    ) {
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            let mut app = app.lock().await;
            if let Err(error) = app.enqueue_kernel_prompt_dispatch(&dispatch) {
                let _ = app.fail_kernel_prompt_dispatch(dispatch, error);
            }
        });
    }

    pub(crate) fn spawn_kernel_prompt_abort_operation(
        app: std::sync::Arc<tokio::sync::Mutex<DaemonApp>>,
        provider_runtime_lanes: ProviderRunOperationLanes,
        dispatch: KernelPromptAbortDispatch,
    ) {
        tokio::spawn(async move {
            let _permit = provider_runtime_lanes
                .acquire(&dispatch.provider_run_id)
                .await;
            loop {
                let mut app = app.lock().await;
                match app.enqueue_kernel_prompt_abort(&dispatch) {
                    Ok(()) => break,
                    Err(_) if app.structured_prompt_io_in_flight(&dispatch.provider_run_id) => {
                        drop(app);
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        continue;
                    }
                    Err(error) => {
                        let _ = app.fail_kernel_prompt_abort(dispatch, error);
                        return;
                    }
                }
            }
        });
    }

    pub fn complete_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCompletion, DaemonError> {
        let target_agent = self.agents.get_agent(agent_id)?;
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            let remote_provider_run_id = match self.block_on_relay_future(
                send_peer_request_via_temporary_connection(
                    &self.config,
                    ClientTarget {
                        daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::CompleteLeasedPrompt {
                        leased_agent_id: remote_execution.leased_agent_id.clone(),
                    },
                ),
            )? {
                RelayPeerResponse::LeasedPromptCompleted {
                    provider_run_id, ..
                } => provider_run_id,
                other => {
                    return Err(DaemonError::LocalTransport {
                        operation: "complete remote prompt",
                        message: format!("unexpected remote prompt completion response: {other:?}"),
                    });
                }
            };
            let (_session, completed) = self
                .sessions
                .complete_active_prompt_only(session_id, agent_id)?;
            let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);
            self.record_assistant_message_completion(
                session_id,
                remote_provider_run_id
                    .as_deref()
                    .unwrap_or("remote-provider-run-completed"),
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            let started_next = if self
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
                self.sync_focused_provider_run_if_idle(session_id)?;
            }
            return Ok(PromptCompletion {
                completed,
                started_next,
            });
        }
        let (_session, completed) = self
            .sessions
            .complete_active_prompt_only(session_id, agent_id)?;
        if !flow_control::prompt_completion_recorded(self, provider_run_id.unwrap_or(agent_id)) {
            let recipient_attachment_ids = self.attachments.list_session_attachment_ids(session_id);
            let completion_provider_run_id = provider_run_id.unwrap_or("provider-run-completed");
            self.record_assistant_message_completion(
                session_id,
                completion_provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completed.id()),
                crate::session::unix_epoch_ms(),
            );
            flow_control::mark_prompt_completion_recorded(self, completion_provider_run_id);
        }
        crate::scheduler::runtime::on_workflow_prompt_completed(
            self,
            session_id,
            &completed,
            provider_run_id,
        )?;
        if let Some(provider_run_id) = provider_run_id {
            flow_control::clear_prompt_activity(self, provider_run_id);
        }
        let started_next = if self
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
        self.kernel_agents()
            .cancel_active_prompt(session_id, attachment_id)
    }

    pub(crate) fn finish_kernel_prompt_abort(
        &mut self,
        session_id: String,
        provider_run_id: String,
        result: Result<(), DaemonError>,
    ) -> Result<(), DaemonError> {
        if let Err(error) = result {
            self.record_notice(
                &session_id,
                Some(&provider_run_id),
                self.attachments.list_session_attachment_ids(&session_id),
                format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
            );
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn enqueue_kernel_prompt_abort(
        &mut self,
        dispatch: &KernelPromptAbortDispatch,
    ) -> Result<(), DaemonError> {
        self.reap_structured_prompt_jobs();
        self.prepare_provider_prompt_dispatch(&dispatch.session_id, &dispatch.provider_run_id)?;
        self.providers.enqueue_structured_prompt_abort(
            dispatch.session_id.clone(),
            dispatch.provider_run_id.clone(),
        )
    }

    pub(crate) fn fail_kernel_prompt_abort(
        &mut self,
        dispatch: KernelPromptAbortDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        self.record_notice(
            &dispatch.session_id,
            Some(&dispatch.provider_run_id),
            self.attachments
                .list_session_attachment_ids(&dispatch.session_id),
            format!("Prompt cancellation dispatch failed after acknowledgement: {error}"),
        );
        Err(error)
    }

    pub(crate) fn structured_prompt_io_in_flight(&self, provider_run_id: &str) -> bool {
        self.providers
            .structured_prompt_io_in_flight(provider_run_id)
    }

    pub(crate) fn reap_structured_prompt_jobs(&mut self) {
        self.providers
            .apply_finished_provider_run_selection_sync_jobs();
        let finished_jobs = self
            .providers
            .drain_finished_structured_prompt_submit_jobs();
        for finished in finished_jobs {
            let _ = self.finish_kernel_prompt_dispatch(
                finished.session_id,
                finished.provider_run_id,
                finished.agent_id,
                finished.result,
            );
        }
        let finished_jobs = self.providers.drain_finished_structured_prompt_abort_jobs();
        for finished in finished_jobs {
            let _ = self.finish_kernel_prompt_abort(
                finished.session_id,
                finished.provider_run_id,
                finished.result,
            );
        }
    }

    pub(crate) fn cancel_active_prompt_for_runtime(
        &mut self,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        let target_agent_id = self
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
        let target_agent = self.agents.get_agent(agent_id)?;
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            match self.block_on_relay_future(send_peer_request_via_temporary_connection(
                &self.config,
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
                .sessions
                .begin_cancelling_active_prompt(session_id, agent_id)?;
            let recipients = match attachment_id {
                Some(attachment_id) => self.other_attachment_ids(session_id, attachment_id),
                None => self.attachments.list_session_attachment_ids(session_id),
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
            self.record_notice(session_id, None, recipients, message);
            return Ok(PromptCancellation {
                prompt,
                started_next: None,
            });
        }
        let provider_run_id = self
            .providers
            .get_run_for_agent(session_id, agent_id)
            .map(|run| run.id().to_string())
            .ok_or_else(|| DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            })?;
        let provider_run = self.ensure_provider_run_in_session(session_id, &provider_run_id)?;

        let uses_structured_prompt_io = self.providers.run_uses_structured_prompt_io(&provider_run);
        if !uses_structured_prompt_io {
            self.send_provider_input(
                session_id,
                &provider_run_id,
                attachment_id.unwrap_or(active_prompt.source_attachment_id()),
                b"\x03",
            )?;
        }

        let (_session, prompt) = self
            .sessions
            .begin_cancelling_active_prompt(session_id, agent_id)?;
        flow_control::note_prompt_settlement_requested(self, &provider_run_id);
        if uses_structured_prompt_io {
            self.providers
                .enqueue_structured_prompt_abort(session_id.to_string(), provider_run_id.clone())?;
        }
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
        agent_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        loop {
            let next_candidate = self
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
                .ensure_prompt_provider_run_for_agent(session_id, &target_agent_id)
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

            let (_session, next_candidate) = self
                .sessions
                .activate_next_queued_prompt(session_id, &target_agent_id)?;
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
                        let cancelled = self
                            .sessions
                            .cancel_active_prompt(session_id, &target_agent_id)?
                            .1;
                        crate::scheduler::runtime::on_workflow_prompt_cancelled(
                            self, session_id, &cancelled,
                        )?;
                        flow_control::clear_prompt_activity(self, &provider_run_id);
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
                    flow_control::note_prompt_started(self, &provider_run_id);
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
            flow_control::note_prompt_started(self, &provider_run_id);
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
                .sessions
                .peek_next_queued_prompt(session_id, agent_id)?;
            let Some(peeked) = next_candidate else {
                return Ok(None);
            };
            let is_workflow_prompt = crate::scheduler::runtime::is_workflow_prompt_attachment(
                peeked.source_attachment_id(),
            );
            if let Err(error) =
                self.ensure_attachment_in_session(session_id, peeked.source_attachment_id())
            {
                if !is_workflow_prompt {
                    self.record_notice(
                        session_id,
                        None,
                        self.attachments.list_session_attachment_ids(session_id),
                        format!(
                            "Skipped queued prompt `{}` because its source attachment is no longer active: {}",
                            peeked.id(),
                            error
                        ),
                    );
                    let _ = self
                        .sessions
                        .activate_next_queued_prompt(session_id, agent_id)?;
                    continue;
                }
            }
            let response = self.block_on_relay_future(send_peer_request_via_temporary_connection(
                &self.config,
                ClientTarget {
                    daemon_id: Some(worker_kernel_id.to_string()),
                    daemon_alias: None,
                },
                RelayPeerRequest::SubmitLeasedPrompt {
                    leased_agent_id: leased_agent_id.to_string(),
                    prompt: peeked.prompt().to_string(),
                    attachments: self.serialize_remote_prompt_attachments(peeked.attachments())?,
                    workflow_context: if is_workflow_prompt {
                        Some(self.remote_workflow_turn_context_for_prompt(
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
                .sessions
                .activate_next_queued_prompt(session_id, agent_id)?;
            let Some(active) = next_candidate else {
                continue;
            };
            self.echo_prompt_to_other_attachments(
                session_id,
                &remote_provider_run_id,
                active.source_attachment_id(),
                active.prompt(),
                active.attachments(),
            );
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
            return Ok(Some(active));
        }
    }

    pub(crate) fn serialize_remote_prompt_attachments(
        &self,
        attachments: &[PromptAttachment],
    ) -> Result<Vec<RelayPromptAttachment>, DaemonError> {
        attachments
            .iter()
            .map(|attachment| {
                let local_path = attachment
                    .url()
                    .strip_prefix("file://localhost")
                    .or_else(|| attachment.url().strip_prefix("file://"))
                    .filter(|path| path.starts_with('/'));
                if let Some(local_path) = local_path {
                    let bytes =
                        fs::read(local_path).map_err(|error| DaemonError::LocalTransport {
                            operation: "read remote prompt attachment",
                            message: error.to_string(),
                        })?;
                    return Ok(RelayPromptAttachment {
                        url: attachment.url().to_string(),
                        mime: attachment.mime().to_string(),
                        filename: attachment.filename().map(str::to_string),
                        contents_base64: Some(
                            base64::engine::general_purpose::STANDARD.encode(bytes),
                        ),
                    });
                }
                Ok(RelayPromptAttachment {
                    url: attachment.url().to_string(),
                    mime: attachment.mime().to_string(),
                    filename: attachment.filename().map(str::to_string),
                    contents_base64: None,
                })
            })
            .collect()
    }

    pub(crate) fn reconcile_provider_run_exit(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<bool, DaemonError> {
        let provider_run = self.ensure_provider_run_in_session(session_id, provider_run_id)?;
        let agent_id = provider_run
            .agent_instance_id()
            .map(str::to_string)
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?;
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
            .active_prompt_for_agent(&agent_id)
            .is_some();
        let ended_run =
            self.providers
                .mark_run_ended(&mut self.sessions, session_id, provider_run_id)?;
        let _ = self.remove_tracked_provider_process_for_run(provider_run_id)?;

        if had_active_prompt {
            let active_prompt_status = self
                .sessions
                .get_session(session_id)?
                .active_prompt_for_agent(&agent_id)
                .map(|prompt| prompt.status());
            if active_prompt_status == Some(PromptStatus::Cancelling) {
                let cancelled = self
                    .sessions
                    .finalize_active_prompt_cancellation(session_id, &agent_id)?
                    .1;
                crate::scheduler::runtime::on_workflow_prompt_cancelled(
                    self, session_id, &cancelled,
                )?;
            } else {
                let completed = self
                    .sessions
                    .complete_active_prompt_only(session_id, &agent_id)?
                    .1;
                crate::scheduler::runtime::on_workflow_prompt_completed(
                    self,
                    session_id,
                    &completed,
                    Some(provider_run_id),
                )?;
            }
            flow_control::clear_prompt_activity(self, provider_run_id);
        }
        self.providers.clear_runtime(provider_run_id);
        let started_next = if had_active_prompt {
            self.advance_next_queued_prompt(session_id, &agent_id)?
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
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        let (_session, prompt) = self
            .sessions
            .finalize_active_prompt_cancellation(session_id, agent_id)?;
        crate::scheduler::runtime::on_workflow_prompt_cancelled(self, session_id, &prompt)?;
        if let Some(provider_run_id) = provider_run_id {
            flow_control::clear_prompt_activity(self, provider_run_id);
        }
        let started_next = if self
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
            self.sync_focused_provider_run_if_idle(session_id)?;
        }

        Ok(PromptCancellation {
            prompt,
            started_next,
        })
    }
}
