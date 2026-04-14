use super::super::prompt_lifecycle::{
    KernelPreparedPromptSubmission, KernelPromptAbortDispatch, KernelPromptAdmission,
    KernelPromptCancellation, KernelPromptDispatch, KernelPromptOwnerSubmission,
    KernelPromptSubmission, KernelRemotePromptDispatch,
};
use super::{select_next_queued_prompt_candidate, KernelAgentService};
use crate::agent::RemoteAgentBinding;
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
        let target_agent_id = match target_agent_id {
            Some(target_agent_id) => target_agent_id.to_string(),
            None => self
                .app
                .sessions()
                .get_session(session_id)?
                .focused_agent_id()
                .ok_or_else(|| DaemonError::AgentNotFound {
                    agent_id: "no focused agent".to_string(),
                })?
                .to_string(),
        };
        let prepared_prompt = PromptQueueItem::new(
            self.app.sessions_mut().reserve_prompt_id(),
            attachment_id,
            &target_agent_id,
            prompt,
            PromptStatus::Queued,
        )
        .with_attachments(attachments);
        let submitted = self.submit_prepared_prompt_for_kernel(KernelPreparedPromptSubmission {
            session_id: session_id.to_string(),
            prompt: prepared_prompt,
            force_queue: false,
        })?;
        let outcome = submitted.outcome;
        self.finish_compat_prompt_dispatch(submitted.dispatch)?;
        self.finish_compat_remote_prompt_dispatch(submitted.remote_dispatch)?;
        self.app.publish_session_projection(session_id)?;
        Ok(outcome)
    }

    fn finish_compat_prompt_dispatch(
        &mut self,
        dispatch: Option<KernelPromptDispatch>,
    ) -> Result<(), DaemonError> {
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        if let Err(error) = self.app.enqueue_kernel_prompt_dispatch(&dispatch) {
            self.app.fail_kernel_prompt_dispatch(dispatch, error)?;
        }
        Ok(())
    }

    fn finish_compat_remote_prompt_dispatch(
        &mut self,
        dispatch: Option<KernelRemotePromptDispatch>,
    ) -> Result<(), DaemonError> {
        let Some(dispatch) = dispatch else {
            return Ok(());
        };
        let attachments = self
            .app
            .serialize_remote_prompt_attachments(&dispatch.attachments)?;
        let result =
            match self
                .app
                .block_on_relay_future(send_peer_request_via_temporary_connection(
                    self.app.config(),
                    ClientTarget {
                        daemon_id: Some(dispatch.worker_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    RelayPeerRequest::SubmitLeasedPrompt {
                        leased_agent_id: dispatch.leased_agent_id.clone(),
                        prompt: dispatch.prompt.clone(),
                        attachments,
                        workflow_context: dispatch.workflow_context.clone(),
                    },
                )) {
                Ok(RelayPeerResponse::LeasedPromptSubmitted {
                    provider_run_id, ..
                }) => Ok(provider_run_id),
                Ok(other) => Err(DaemonError::LocalTransport {
                    operation: "submit remote prepared prompt",
                    message: format!("unexpected remote prompt response: {other:?}"),
                }),
                Err(error) => Err(error),
            };
        self.app
            .finish_kernel_remote_prompt_dispatch(dispatch, result)
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

        let dispatch =
            if uses_structured_prompt_io {
                Some(KernelPromptAbortDispatch {
                    session_id: session_id.clone(),
                    provider_run_id: provider_run_id.clone(),
                })
            } else {
                crate::app::terminal_input::ProviderTerminalInput::new(self.app)
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
    pub(crate) fn acquire_provider_prompt_claim(
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
            crate::app::terminal_input::ProviderTerminalInput::new(self.app).send_provider_input(
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
}
