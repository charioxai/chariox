use crate::agent::RemoteAgentBinding;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::execution_lease::RemoteWorkflowTurnContext;
use crate::provider::ProviderRunState;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptQueueItem,
    PromptSubmissionOutcome,
};
use crate::transport::flow_control;
use crate::transport::relay_peer::RelayPromptAttachment;
use base64::Engine;
use std::fs;

pub(crate) struct KernelPromptSubmission {
    pub(crate) outcome: PromptSubmissionOutcome,
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) dispatch: Option<KernelPromptDispatch>,
    pub(crate) remote_dispatch: Option<KernelRemotePromptDispatch>,
}

pub(crate) struct KernelPreparedPromptSubmission {
    pub(crate) session_id: String,
    pub(crate) prompt: PromptQueueItem,
    pub(crate) force_queue: bool,
}

pub(crate) struct KernelPromptAdmission {
    pub(crate) session_id: String,
    pub(crate) attachment_id: String,
    pub(crate) target_agent_id: String,
    pub(crate) prompt: PromptQueueItem,
    pub(crate) force_queue: bool,
    pub(crate) provider_run_id: Option<String>,
    pub(crate) remote_execution: Option<RemoteAgentBinding>,
    pub(crate) provider_run_is_starting: bool,
}

pub(crate) struct KernelPromptOwnerSubmission {
    pub(crate) admission: KernelPromptAdmission,
    pub(crate) outcome: PromptSubmissionOutcome,
}

pub(crate) struct KernelPromptDispatch {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) source_attachment_id: String,
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<PromptAttachment>,
}

pub(crate) struct KernelRemotePromptDispatch {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) worker_kernel_id: String,
    pub(crate) leased_agent_id: String,
    pub(crate) source_attachment_id: String,
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<PromptAttachment>,
    pub(crate) workflow_context: Option<RemoteWorkflowTurnContext>,
}

pub(crate) struct KernelPromptCancellation {
    pub(crate) cancellation: PromptCancellation,
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) dispatch: Option<KernelPromptAbortDispatch>,
}

pub(crate) struct KernelPromptAbortDispatch {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
}

impl DaemonApp {
    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        crate::app::KernelAgentService::new(self).submit_prompt(
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            attachments,
        )
    }

    pub(crate) fn prepare_provider_prompt_dispatch(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
    ) -> Result<crate::provider::RuntimeProviderRun, DaemonError> {
        let _ = super::provider_runtime::ProviderRunLivenessRuntime::new(self)
            .reconcile_provider_run_exit(session_id, provider_run_id)?;
        let provider_run = crate::app::ProviderRunReadService::new(self)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;
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
            crate::app::KernelAgentService::new(self).cancel_active_after_prompt_start_failure(
                &session_id,
                &agent_id,
                &provider_run_id,
            );
            let _ = crate::app::KernelSessionReadService::new(self).session_snapshot(&session_id);
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
        self.echo_prompt_to_other_attachments(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            &dispatch.prompt,
            &dispatch.attachments,
        );
        let provider_run =
            self.prepare_provider_prompt_dispatch(&dispatch.session_id, &dispatch.provider_run_id)?;
        if self.providers.run_uses_structured_prompt_io(&provider_run) {
            flow_control::note_prompt_started(self, &dispatch.provider_run_id);
            return self.providers.enqueue_structured_prompt_submit(
                dispatch.session_id.clone(),
                dispatch.provider_run_id.clone(),
                dispatch.agent_id.clone(),
                &provider_run,
                &dispatch.prompt,
                &dispatch.attachments,
            );
        }
        crate::app::terminal_input::ProviderTerminalInput::new(self).send_provider_input(
            &dispatch.session_id,
            &dispatch.provider_run_id,
            &dispatch.source_attachment_id,
            dispatch.prompt.as_bytes(),
        )?;
        flow_control::note_prompt_started(self, &dispatch.provider_run_id);
        Ok(())
    }

    pub(crate) fn fail_kernel_prompt_dispatch(
        &mut self,
        dispatch: KernelPromptDispatch,
        error: DaemonError,
    ) -> Result<(), DaemonError> {
        let _ =
            self.prompt_owner_cancel_active_prompt_only(&dispatch.session_id, &dispatch.agent_id);
        flow_control::clear_prompt_activity(self, &dispatch.provider_run_id);
        let _ =
            crate::app::KernelSessionReadService::new(self).session_snapshot(&dispatch.session_id);
        self.record_notice(
            &dispatch.session_id,
            Some(&dispatch.provider_run_id),
            self.attachments
                .list_session_attachment_ids(&dispatch.session_id),
            format!("Prompt dispatch failed after acknowledgement: {error}"),
        );
        Err(error)
    }

    pub(crate) fn finish_kernel_remote_prompt_dispatch(
        &mut self,
        dispatch: KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        match result {
            Ok(remote_provider_run_id) => {
                self.echo_prompt_to_other_attachments(
                    &dispatch.session_id,
                    &remote_provider_run_id,
                    &dispatch.source_attachment_id,
                    &dispatch.prompt,
                    &dispatch.attachments,
                );
                Ok(())
            }
            Err(error) => {
                let _ = self.prompt_owner_cancel_active_prompt_only(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                );
                let _ = crate::app::KernelSessionReadService::new(self)
                    .session_snapshot(&dispatch.session_id);
                self.record_notice(
                    &dispatch.session_id,
                    None,
                    self.attachments
                        .list_session_attachment_ids(&dispatch.session_id),
                    format!("Remote prompt dispatch failed after acknowledgement: {error}"),
                );
                Err(error)
            }
        }
    }

    pub(crate) fn complete_active_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCompletion, DaemonError> {
        crate::app::KernelAgentService::new(self).complete_active_prompt(
            session_id,
            agent_id,
            provider_run_id,
        )
    }

    pub(crate) fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        crate::app::KernelAgentService::new(self).cancel_active_prompt(session_id, attachment_id)
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
        crate::app::KernelAgentService::new(self).cancel_active_prompt_for_runtime(session_id)
    }

    pub(crate) fn cancel_active_prompt_internal(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        crate::app::KernelAgentService::new(self).cancel_active_prompt_internal(
            session_id,
            agent_id,
            attachment_id,
        )
    }

    pub(crate) fn advance_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let expected_next = self
            .agent_runtime_projection_store()
            .next_queued_prompt(session_id, agent_id);
        crate::app::KernelAgentService::new(self).advance_next_queued_prompt(
            session_id,
            agent_id,
            expected_next.as_ref(),
        )
    }

    pub(crate) fn advance_next_queued_prompt_remote(
        &mut self,
        session_id: &str,
        agent_id: &str,
        worker_kernel_id: &str,
        leased_agent_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        crate::app::KernelAgentService::new(self).advance_next_queued_prompt_remote(
            session_id,
            agent_id,
            worker_kernel_id,
            leased_agent_id,
            None,
        )
    }

    pub(crate) fn serialize_remote_prompt_attachments(
        &self,
        attachments: &[PromptAttachment],
    ) -> Result<Vec<RelayPromptAttachment>, DaemonError> {
        serialize_remote_prompt_attachments(attachments)
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        crate::app::KernelAgentService::new(self).finalize_active_prompt_cancellation(
            session_id,
            agent_id,
            provider_run_id,
        )
    }
}

pub(crate) fn serialize_remote_prompt_attachments(
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
                let bytes = fs::read(local_path).map_err(|error| DaemonError::LocalTransport {
                    operation: "read remote prompt attachment",
                    message: error.to_string(),
                })?;
                return Ok(RelayPromptAttachment {
                    url: attachment.url().to_string(),
                    mime: attachment.mime().to_string(),
                    filename: attachment.filename().map(str::to_string),
                    contents_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
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
