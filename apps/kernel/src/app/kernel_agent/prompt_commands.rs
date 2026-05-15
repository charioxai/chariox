use super::super::prompt_lifecycle::{
    KernelPreparedPromptSubmission, KernelPromptAdmission, KernelPromptDispatch,
    KernelPromptOwnerSubmission, KernelPromptSubmission, KernelRemotePromptDispatch,
};
use super::KernelAgentService;
use crate::error::DaemonError;
use crate::provider::ProviderRunState;
use crate::session::{PromptAttachment, PromptQueueItem, PromptStatus, PromptSubmissionOutcome};

mod cancellation;
mod completion;
mod queue;
mod remote;

impl<'a> KernelAgentService<'a> {
    pub(crate) fn submit_prepared_prompt_for_kernel(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        let admission = self.prepare_prompt_admission(prepared)?;
        self.spawn_prompt_history_append(&admission)?;
        let submitted = self.submit_admitted_prompt_to_owner(admission)?;
        let (dispatch, remote_dispatch) = self.prepare_prompt_submission_effects(&submitted)?;
        let session = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&submitted.admission.session_id)?;
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
        crate::app::KernelSessionReadService::new(self.app)
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
        crate::app::KernelSessionReadService::new(self.app).session_snapshot(session_id)?;
        Ok(outcome)
    }

    pub(crate) fn record_native_prompt_started(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: &str,
        prompt: &str,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        crate::app::KernelSessionReadService::new(self.app)
            .ensure_attachment_in_session(session_id, attachment_id)?;
        let prepared_prompt = PromptQueueItem::new(
            self.app.sessions_mut().reserve_prompt_id(),
            attachment_id,
            target_agent_id,
            prompt,
            PromptStatus::Queued,
        );
        let admission = self.prepare_prompt_admission(KernelPreparedPromptSubmission {
            session_id: session_id.to_string(),
            prompt: prepared_prompt,
            force_queue: false,
        })?;
        if admission.remote_execution.is_some() {
            return Err(DaemonError::LocalTransport {
                operation: "record native prompt",
                message: "native provider prompt recording requires a local provider run"
                    .to_string(),
            });
        }
        self.spawn_prompt_history_append(&admission)?;
        let submitted = self.submit_admitted_prompt_to_owner(admission)?;
        let provider_run_id = submitted.admission.provider_run_id.clone();
        let (dispatch, _) = self.prepare_local_prompt_submission_effects(&submitted)?;
        if matches!(submitted.outcome, PromptSubmissionOutcome::Started { .. }) {
            if let Some(provider_run_id) = provider_run_id.or_else(|| {
                dispatch
                    .as_ref()
                    .map(|dispatch| dispatch.provider_run_id.clone())
            }) {
                crate::transport::flow_control::note_prompt_started(self.app, &provider_run_id);
            }
        }
        let _ = crate::app::KernelSessionReadService::new(self.app)
            .session_snapshot(&submitted.admission.session_id)?;
        Ok(submitted.outcome)
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

    fn prepare_prompt_admission(
        &mut self,
        prepared: KernelPreparedPromptSubmission,
    ) -> Result<KernelPromptAdmission, DaemonError> {
        let session_id = prepared.session_id;
        let attachment_id = prepared.prompt.source_attachment_id().to_string();
        let target_agent_id = prepared.prompt.target_agent_id().to_string();
        crate::app::KernelSessionReadService::new(self.app)
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
                prompt_id: prompt.id().to_string(),
                worker_kernel_id: remote_execution.worker_kernel_id.clone(),
                leased_agent_id: remote_execution.leased_agent_id.clone(),
                relay_url: remote_execution.relay_url.clone(),
                relay_token: remote_execution.relay_token.clone(),
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
                dispatch = Some(KernelPromptDispatch {
                    session_id: submitted.admission.session_id.clone(),
                    provider_run_id: provider_run_id.to_string(),
                    agent_id: submitted.admission.target_agent_id.clone(),
                    prompt_id: prompt.id().to_string(),
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
}
