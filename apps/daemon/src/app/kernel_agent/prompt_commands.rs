use super::super::prompt_lifecycle::{
    KernelPreparedPromptSubmission, KernelPromptAbortDispatch, KernelPromptAdmission,
    KernelPromptCancellation, KernelPromptDispatch, KernelPromptOwnerSubmission,
    KernelPromptSubmission, KernelRemotePromptDispatch,
};
use super::KernelAgentService;
use crate::agent::RemoteAgentBinding;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::{
    PromptCancellation, PromptCompletion, PromptQueueItem, PromptStatus, PromptSubmissionOutcome,
};
use crate::transport::flow_control;
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

enum KernelPromptCancellationAdmission {
    Remote {
        session_id: String,
        target_agent_id: String,
        attachment_id: String,
    },
    AlreadyCancelling {
        session_id: String,
        active_prompt: PromptQueueItem,
    },
    Local {
        session_id: String,
        target_agent_id: String,
        attachment_id: String,
        active_prompt: PromptQueueItem,
        provider_run_id: String,
        uses_structured_prompt_io: bool,
    },
}

struct KernelPromptOwnerCancellation {
    session_id: String,
    attachment_id: String,
    active_prompt: PromptQueueItem,
    provider_run_id: String,
    prompt: PromptQueueItem,
    dispatch: Option<KernelPromptAbortDispatch>,
}

enum KernelPromptCompletionAdmission {
    Remote {
        session_id: String,
        agent_id: String,
        remote_execution: RemoteAgentBinding,
        next_queued_prompt: Option<PromptQueueItem>,
    },
    Local {
        session_id: String,
        agent_id: String,
        provider_run_id: Option<String>,
        next_queued_prompt: Option<PromptQueueItem>,
    },
}

struct KernelPromptOwnerCompletion {
    session_id: String,
    agent_id: String,
    completed: PromptQueueItem,
    provider_run_id: Option<String>,
    remote_execution: Option<RemoteAgentBinding>,
    remote_provider_run_id: Option<String>,
    next_queued_prompt: Option<PromptQueueItem>,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn submit_prepared_prompt_for_kernel(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        let admission = self.prepare_prompt_admission(prepared)?;
        self.spawn_prompt_history_append(&admission)?;
        let submitted = self.submit_admitted_prompt_to_owner(admission)?;
        let (dispatch, remote_dispatch) = self.prepare_prompt_submission_effects(&submitted)?;
        let session = self
            .app
            .local_api_session_snapshot(&submitted.admission.session_id)?;
        Ok(KernelPromptSubmission {
            outcome: submitted.outcome,
            session,
            dispatch,
            remote_dispatch,
        })
    }

    fn prepare_prompt_admission(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptAdmission, DaemonError> {
        let session_id = prepared.session_id;
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        self.app
            .ensure_attachment_in_session(&session_id, &attachment_id)?;

        let target_agent = self.app.agents.get_agent(&target_agent_id)?;
        let remote_execution = target_agent.remote_execution().cloned();
        let (provider_run_id, provider_run_is_starting) = if remote_execution.is_some() {
            (None, false)
        } else {
            let queued_while_active = self
                .app
                .prompt_owner_active_prompt_for_agent(&session_id, &target_agent_id)?
                .is_some();
            let provider_run_id = if queued_while_active {
                self.app
                    .providers
                    .get_run_for_agent(&session_id, &target_agent_id)
                    .map(|run| run.id().to_string())
            } else {
                Some(
                    self.app
                        .ensure_prompt_provider_run_for_agent(&session_id, &target_agent_id)?,
                )
            };
            let provider_run_is_starting = provider_run_id
                .as_deref()
                .and_then(|provider_run_id| self.app.providers.get_run(provider_run_id).ok())
                .is_some_and(|run| run.state() == ProviderRunState::Starting);
            (provider_run_id, provider_run_is_starting)
        };

        Ok(KernelPromptAdmission {
            session_id,
            attachment_id,
            target_agent_id,
            prompt: prepared.prompt,
            force_queue: prepared.force_queue,
            provider_run_id,
            remote_execution,
            provider_run_is_starting,
        })
    }

    fn spawn_prompt_history_append(
        &self,
        admission: &KernelPromptAdmission,
    ) -> Result<(), DaemonError> {
        self.app.spawn_user_prompt_history_append(
            &admission.session_id,
            &admission.attachment_id,
            &admission.target_agent_id,
            admission.prompt.prompt(),
            admission.prompt.attachments(),
        )
    }

    fn submit_admitted_prompt_to_owner(
        &mut self,
        admission: KernelPromptAdmission,
    ) -> Result<KernelPromptOwnerSubmission, DaemonError> {
        let outcome = self.app.prompt_owner_submit_prepared_prompt(
            &admission.session_id,
            admission.prompt.clone(),
            admission.force_queue || admission.provider_run_is_starting,
        )?;
        Ok(KernelPromptOwnerSubmission { admission, outcome })
    }

    fn prepare_prompt_submission_effects(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
    ) -> Result<
        (
            Option<KernelPromptDispatch>,
            Option<KernelRemotePromptDispatch>,
        ),
        DaemonError,
    > {
        if submitted.admission.remote_execution.is_some() {
            return self.prepare_remote_prompt_submission_effects(submitted);
        }
        self.prepare_local_prompt_submission_effects(submitted)
    }

    fn prepare_remote_prompt_submission_effects(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
    ) -> Result<
        (
            Option<KernelPromptDispatch>,
            Option<KernelRemotePromptDispatch>,
        ),
        DaemonError,
    > {
        let mut remote_dispatch = None;
        if let (Some(remote_execution), PromptSubmissionOutcome::Started { prompt }) = (
            submitted.admission.remote_execution.as_ref(),
            &submitted.outcome,
        ) {
            remote_dispatch = Some(KernelRemotePromptDispatch {
                session_id: submitted.admission.session_id.clone(),
                agent_id: submitted.admission.target_agent_id.clone(),
                worker_kernel_id: remote_execution.worker_kernel_id.clone(),
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                source_attachment_id: prompt.source_attachment_id().to_string(),
                prompt: prompt.prompt().to_string(),
                attachments: prompt.attachments().to_vec(),
                workflow_context: None,
            });
        }
        Ok((None, remote_dispatch))
    }

    fn prepare_local_prompt_submission_effects(
        &mut self,
        submitted: &KernelPromptOwnerSubmission,
    ) -> Result<
        (
            Option<KernelPromptDispatch>,
            Option<KernelRemotePromptDispatch>,
        ),
        DaemonError,
    > {
        let mut dispatch = None;
        match &submitted.outcome {
            PromptSubmissionOutcome::Started { prompt } => {
                let provider_run_id =
                    submitted
                        .admission
                        .provider_run_id
                        .as_deref()
                        .ok_or_else(|| DaemonError::NoActiveProviderRun {
                            session_id: submitted.admission.session_id.clone(),
                        })?;
                self.app.echo_prompt_to_other_attachments(
                    &submitted.admission.session_id,
                    provider_run_id,
                    prompt.source_attachment_id(),
                    prompt.prompt(),
                    prompt.attachments(),
                );
                if let Err(error) = self.acquire_provider_prompt_claim(
                    &submitted.admission.session_id,
                    provider_run_id,
                    &submitted.admission.target_agent_id,
                    prompt.source_attachment_id(),
                ) {
                    self.cancel_active_after_prompt_start_failure(
                        &submitted.admission.session_id,
                        &submitted.admission.target_agent_id,
                        provider_run_id,
                    );
                    return Err(error);
                }
                dispatch = Some(KernelPromptDispatch {
                    session_id: submitted.admission.session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: submitted.admission.target_agent_id.clone(),
                    source_attachment_id: prompt.source_attachment_id().to_string(),
                    prompt: prompt.prompt().to_string(),
                    attachments: prompt.attachments().to_vec(),
                });
            }
            PromptSubmissionOutcome::Queued { prompt } => {
                let queue_depth = self
                    .app
                    .prompt_owner_queued_prompt_count_for_agent(
                        &submitted.admission.session_id,
                        &submitted.admission.target_agent_id,
                    )
                    .unwrap_or(0);
                if let Some(provider_run_id) = submitted.admission.provider_run_id.as_deref() {
                    self.app.echo_prompt_to_other_attachments(
                        &submitted.admission.session_id,
                        provider_run_id,
                        prompt.source_attachment_id(),
                        prompt.prompt(),
                        prompt.attachments(),
                    );
                }
                self.app.record_notice(
                    &submitted.admission.session_id,
                    submitted.admission.provider_run_id.as_deref(),
                    self.app.other_attachment_ids(
                        &submitted.admission.session_id,
                        &submitted.admission.attachment_id,
                    ),
                    format!(
                        "A queued message from attachment `{}` was added to agent `{}` in session `{}` as `{}`. Queue depth is now {}.",
                        submitted.admission.attachment_id,
                        submitted.admission.target_agent_id,
                        submitted.admission.session_id,
                        prompt.id(),
                        queue_depth
                    ),
                );
            }
        }
        Ok((dispatch, None))
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
        let admission = self.prepare_prompt_completion_admission(
            session_id,
            agent_id,
            provider_run_id,
            next_queued_prompt,
        )?;
        let completion = match admission {
            KernelPromptCompletionAdmission::Remote { .. } => {
                let completed = self.complete_remote_prompt_from_admission(admission)?;
                self.finish_remote_prompt_completion(completed)?
            }
            KernelPromptCompletionAdmission::Local { .. } => {
                let completed = self.complete_local_prompt_from_admission(admission)?;
                self.finish_local_prompt_completion(completed)?
            }
        };
        Ok(completion)
    }

    fn prepare_prompt_completion_admission(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: Option<&PromptQueueItem>,
    ) -> Result<KernelPromptCompletionAdmission, DaemonError> {
        let target_agent = self.app.agents.get_agent(agent_id)?;
        let next_queued_prompt = next_queued_prompt.cloned();
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            return Ok(KernelPromptCompletionAdmission::Remote {
                session_id: session_id.to_string(),
                agent_id: agent_id.to_string(),
                remote_execution,
                next_queued_prompt,
            });
        }
        Ok(KernelPromptCompletionAdmission::Local {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            provider_run_id: provider_run_id.map(str::to_string),
            next_queued_prompt,
        })
    }

    fn complete_remote_prompt_from_admission(
        &mut self,
        admission: KernelPromptCompletionAdmission,
    ) -> Result<KernelPromptOwnerCompletion, DaemonError> {
        let KernelPromptCompletionAdmission::Remote {
            session_id,
            agent_id,
            remote_execution,
            next_queued_prompt,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "complete prompt admission",
                message: "expected remote prompt completion admission".to_string(),
            });
        };

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
                        message: format!("unexpected remote prompt completion response: {other:?}"),
                    });
                }
            };
        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(&session_id, &agent_id)?;
        Ok(KernelPromptOwnerCompletion {
            session_id,
            agent_id,
            completed,
            provider_run_id: None,
            remote_execution: Some(remote_execution),
            remote_provider_run_id,
            next_queued_prompt,
        })
    }

    fn finish_remote_prompt_completion(
        &mut self,
        completion: KernelPromptOwnerCompletion,
    ) -> Result<PromptCompletion, DaemonError> {
        let remote_provider_run_id = completion
            .remote_provider_run_id
            .as_deref()
            .unwrap_or("remote-provider-run-completed");
        let recipient_attachment_ids = self
            .app
            .attachments
            .list_session_attachment_ids(&completion.session_id);
        self.app.record_assistant_message_completion(
            &completion.session_id,
            remote_provider_run_id,
            recipient_attachment_ids,
            &format!("prompt-complete:{}", completion.completed.id()),
            crate::session::unix_epoch_ms(),
        );
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(&completion.session_id, &completion.agent_id)?
            .is_none()
        {
            let remote_execution = completion.remote_execution.as_ref().ok_or_else(|| {
                DaemonError::LocalTransport {
                    operation: "complete remote prompt",
                    message: "missing remote execution binding".to_string(),
                }
            })?;
            self.advance_next_queued_prompt_remote(
                &completion.session_id,
                &completion.agent_id,
                &remote_execution.worker_kernel_id,
                &remote_execution.leased_agent_id,
                completion.next_queued_prompt.as_ref(),
            )?
        } else {
            None
        };
        if started_next.is_none() {
            self.app
                .sync_focused_provider_run_if_idle(&completion.session_id)?;
        }
        self.app
            .publish_session_projection(&completion.session_id)?;

        Ok(PromptCompletion {
            completed: completion.completed,
            started_next,
        })
    }

    fn complete_local_prompt_from_admission(
        &mut self,
        admission: KernelPromptCompletionAdmission,
    ) -> Result<KernelPromptOwnerCompletion, DaemonError> {
        let KernelPromptCompletionAdmission::Local {
            session_id,
            agent_id,
            provider_run_id,
            next_queued_prompt,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "complete prompt admission",
                message: "expected local prompt completion admission".to_string(),
            });
        };

        let completed = self
            .app
            .prompt_owner_complete_active_prompt_only(&session_id, &agent_id)?;
        Ok(KernelPromptOwnerCompletion {
            session_id,
            agent_id,
            completed,
            provider_run_id,
            remote_execution: None,
            remote_provider_run_id: None,
            next_queued_prompt,
        })
    }

    fn finish_local_prompt_completion(
        &mut self,
        completion: KernelPromptOwnerCompletion,
    ) -> Result<PromptCompletion, DaemonError> {
        if !flow_control::prompt_completion_recorded(
            self.app,
            completion
                .provider_run_id
                .as_deref()
                .unwrap_or(&completion.agent_id),
        ) {
            let recipient_attachment_ids = self
                .app
                .attachments
                .list_session_attachment_ids(&completion.session_id);
            let completion_provider_run_id = completion
                .provider_run_id
                .as_deref()
                .unwrap_or("provider-run-completed");
            self.app.record_assistant_message_completion(
                &completion.session_id,
                completion_provider_run_id,
                recipient_attachment_ids,
                &format!("prompt-complete:{}", completion.completed.id()),
                crate::session::unix_epoch_ms(),
            );
            flow_control::mark_prompt_completion_recorded(self.app, completion_provider_run_id);
        }
        self.app.complete_workflow_prompt_from_runtime(
            &completion.session_id,
            &completion.completed,
            completion.provider_run_id.as_deref(),
        )?;
        let completion_provider_run_id = completion.provider_run_id.clone().or_else(|| {
            self.app
                .providers
                .get_run_for_agent(&completion.session_id, &completion.agent_id)
                .map(|run| run.id().to_string())
        });
        if let Some(provider_run_id) = completion_provider_run_id.as_deref() {
            flow_control::clear_prompt_activity(self.app, provider_run_id);
        }
        let started_next = if self
            .app
            .prompt_owner_active_prompt_for_agent(&completion.session_id, &completion.agent_id)?
            .is_none()
        {
            self.advance_next_queued_prompt(
                &completion.session_id,
                &completion.agent_id,
                completion.next_queued_prompt.as_ref(),
            )?
        } else {
            None
        };
        if started_next.is_none() {
            self.app
                .sync_focused_provider_run_if_idle(&completion.session_id)?;
        }
        self.app
            .publish_session_projection(&completion.session_id)?;

        Ok(PromptCompletion {
            completed: completion.completed,
            started_next,
        })
    }

    pub(crate) fn cancel_agent_prompt_for_kernel(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        let admission =
            self.prepare_prompt_cancellation_admission(session_id, target_agent_id, attachment_id)?;
        match admission {
            KernelPromptCancellationAdmission::Remote {
                session_id,
                target_agent_id,
                attachment_id,
            } => self.cancel_remote_agent_prompt(&session_id, &target_agent_id, &attachment_id),
            KernelPromptCancellationAdmission::AlreadyCancelling {
                session_id,
                active_prompt,
            } => self.finish_already_cancelling_prompt(&session_id, active_prompt),
            KernelPromptCancellationAdmission::Local { .. } => {
                let cancelled = self.cancel_local_prompt_from_admission(admission)?;
                self.finish_local_prompt_cancellation(cancelled)
            }
        }
    }

    fn prepare_prompt_cancellation_admission(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellationAdmission, DaemonError> {
        self.app
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let target_agent = self.app.agents.get_agent(target_agent_id)?;
        if target_agent.remote_execution().is_some() {
            return Ok(KernelPromptCancellationAdmission::Remote {
                session_id: session_id.to_string(),
                target_agent_id: target_agent_id.to_string(),
                attachment_id: attachment_id.to_string(),
            });
        }

        let active_prompt = self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, target_agent_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        if active_prompt.status() == PromptStatus::Cancelling {
            return Ok(KernelPromptCancellationAdmission::AlreadyCancelling {
                session_id: session_id.to_string(),
                active_prompt,
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
        let uses_structured_prompt_io = self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run);

        Ok(KernelPromptCancellationAdmission::Local {
            session_id: session_id.to_string(),
            target_agent_id: target_agent_id.to_string(),
            attachment_id: attachment_id.to_string(),
            active_prompt,
            provider_run_id,
            uses_structured_prompt_io,
        })
    }

    fn cancel_remote_agent_prompt(
        &mut self,
        session_id: &str,
        target_agent_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        let cancellation =
            self.cancel_active_prompt_internal(session_id, target_agent_id, Some(attachment_id))?;
        let session = self.app.local_api_session_snapshot(session_id)?;
        Ok(KernelPromptCancellation {
            cancellation,
            session,
            dispatch: None,
        })
    }

    fn finish_already_cancelling_prompt(
        &self,
        session_id: &str,
        active_prompt: PromptQueueItem,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        let session = self.app.local_api_session_snapshot(session_id)?;
        Ok(KernelPromptCancellation {
            cancellation: PromptCancellation {
                prompt: active_prompt,
                started_next: None,
            },
            session,
            dispatch: None,
        })
    }

    fn cancel_local_prompt_from_admission(
        &mut self,
        admission: KernelPromptCancellationAdmission,
    ) -> Result<KernelPromptOwnerCancellation, DaemonError> {
        let KernelPromptCancellationAdmission::Local {
            session_id,
            target_agent_id,
            attachment_id,
            active_prompt,
            provider_run_id,
            uses_structured_prompt_io,
        } = admission
        else {
            return Err(DaemonError::LocalTransport {
                operation: "cancel prompt admission",
                message: "expected local prompt cancellation admission".to_string(),
            });
        };

        let dispatch = if uses_structured_prompt_io {
            Some(KernelPromptAbortDispatch {
                session_id: session_id.clone(),
                provider_run_id: provider_run_id.clone(),
            })
        } else {
            self.app
                .send_provider_input(&session_id, &provider_run_id, &attachment_id, b"\x03")?;
            None
        };

        let prompt = self
            .app
            .prompt_owner_begin_cancelling_active_prompt(&session_id, &target_agent_id)?;
        flow_control::note_prompt_settlement_requested(self.app, &provider_run_id);

        Ok(KernelPromptOwnerCancellation {
            session_id,
            attachment_id,
            active_prompt,
            provider_run_id,
            prompt,
            dispatch,
        })
    }

    fn finish_local_prompt_cancellation(
        &mut self,
        cancelled: KernelPromptOwnerCancellation,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        self.app.record_notice(
            &cancelled.session_id,
            Some(&cancelled.provider_run_id),
            self.app
                .other_attachment_ids(&cancelled.session_id, &cancelled.attachment_id),
            format!(
                "Attachment `{}` requested cancellation of active prompt `{}` on provider run `{}`.",
                cancelled.attachment_id,
                cancelled.active_prompt.id(),
                cancelled.provider_run_id
            ),
        );
        let session = self.app.publish_session_projection(&cancelled.session_id)?;

        Ok(KernelPromptCancellation {
            cancellation: PromptCancellation {
                prompt: cancelled.prompt,
                started_next: None,
            },
            session,
            dispatch: cancelled.dispatch,
        })
    }
}
