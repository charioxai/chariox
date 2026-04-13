use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{
    ProviderRunLivenessReconciliation, ProviderRunOperationLanes, ProviderRunState,
};
use crate::pty::PtyProcessState;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptQueueItem, PromptStatus,
    PromptSubmissionOutcome,
};
use crate::transport::flow_control;
use crate::transport::relay_peer::RelayPromptAttachment;
use base64::Engine;
use std::fs;
use std::time::Duration;

pub(crate) struct KernelPromptSubmission {
    pub(crate) outcome: PromptSubmissionOutcome,
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) dispatch: Option<KernelPromptDispatch>,
}

pub(crate) struct KernelPreparedPromptSubmission {
    pub(crate) session_id: String,
    pub(crate) prompt: PromptQueueItem,
    pub(crate) force_queue: bool,
}

pub(crate) struct KernelPromptDispatch {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) agent_id: String,
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<PromptAttachment>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum ProviderRunExitPromptSettlement {
    FinalizeCancellation,
    CompleteActivePrompt,
    SyncIdleProvider,
}

impl ProviderRunExitPromptSettlement {
    fn from_active_prompt_status(active_prompt_status: Option<PromptStatus>) -> Self {
        match active_prompt_status {
            Some(PromptStatus::Cancelling) => Self::FinalizeCancellation,
            Some(_) => Self::CompleteActivePrompt,
            None => Self::SyncIdleProvider,
        }
    }
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
    pub fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.kernel_agents().submit_prompt(
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
            self.kernel_agents()
                .cancel_active_after_prompt_start_failure(&session_id, &agent_id, &provider_run_id);
            let _ = self.publish_session_projection(&session_id);
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
        let _ = self.publish_session_projection(&dispatch.session_id);
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
        self.kernel_agents()
            .complete_active_prompt(session_id, agent_id, provider_run_id)
    }

    pub(crate) fn complete_active_prompt_for_kernel(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
        next_queued_prompt: Option<&PromptQueueItem>,
    ) -> Result<PromptCompletion, DaemonError> {
        self.kernel_agents().complete_active_prompt_for_kernel(
            session_id,
            agent_id,
            provider_run_id,
            next_queued_prompt,
        )
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
        self.kernel_agents()
            .cancel_active_prompt_for_runtime(session_id)
    }

    pub(crate) fn cancel_active_prompt_internal(
        &mut self,
        session_id: &str,
        agent_id: &str,
        attachment_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        self.kernel_agents()
            .cancel_active_prompt_internal(session_id, agent_id, attachment_id)
    }

    pub(crate) fn advance_next_queued_prompt(
        &mut self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        let expected_next = self
            .agent_runtime_projection_store()
            .next_queued_prompt(session_id, agent_id);
        self.kernel_agents().advance_next_queued_prompt(
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
        self.kernel_agents().advance_next_queued_prompt_remote(
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
        match self.providers.reconcile_run_liveness(
            &mut self.sessions,
            session_id,
            provider_run_id,
            None,
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run) => {
                self.update_provider_run_projection(run);
                let _ = self.remove_tracked_provider_process_for_run(provider_run_id)?;
                return Ok(true);
            }
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::NewlyEnded(_) => return Ok(false),
            ProviderRunLivenessReconciliation::StillRunning(_) => {}
        }

        let had_active_prompt = self
            .sessions
            .get_session(session_id)?
            .active_prompt_for_agent(&agent_id)
            .is_some();
        let active_prompt_status = if had_active_prompt {
            self.sessions
                .get_session(session_id)?
                .active_prompt_for_agent(&agent_id)
                .map(|prompt| prompt.status())
        } else {
            None
        };
        let process_running = match self.pty.poll_process_state(provider_run_id) {
            Ok(PtyProcessState::Running) => true,
            Ok(PtyProcessState::Exited) => false,
            Err(DaemonError::PtyProcessNotFound { .. }) => false,
            Err(error) => return Err(error),
        };
        let ended_run = match self.providers.reconcile_run_liveness(
            &mut self.sessions,
            session_id,
            provider_run_id,
            Some(process_running),
        )? {
            ProviderRunLivenessReconciliation::AlreadyEnded(run)
            | ProviderRunLivenessReconciliation::NewlyEnded(run) => run,
            ProviderRunLivenessReconciliation::ExternalEndpoint(_)
            | ProviderRunLivenessReconciliation::StillRunning(_) => return Ok(false),
        };
        self.update_provider_run_projection(ended_run.clone());
        let _ = self.remove_tracked_provider_process_for_run(provider_run_id)?;

        let started_next = self.settle_provider_exit_prompt_state(
            session_id,
            &agent_id,
            provider_run_id,
            active_prompt_status,
        )?;

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

    fn settle_provider_exit_prompt_state(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: &str,
        active_prompt_status: Option<PromptStatus>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        match ProviderRunExitPromptSettlement::from_active_prompt_status(active_prompt_status) {
            ProviderRunExitPromptSettlement::FinalizeCancellation => Ok(self
                .finalize_active_prompt_cancellation(session_id, agent_id, Some(provider_run_id))?
                .started_next),
            ProviderRunExitPromptSettlement::CompleteActivePrompt => Ok(self
                .complete_active_prompt(session_id, agent_id, Some(provider_run_id))?
                .started_next),
            ProviderRunExitPromptSettlement::SyncIdleProvider => {
                self.sync_focused_provider_run_if_idle(session_id)?;
                Ok(None)
            }
        }
    }

    pub(crate) fn finalize_active_prompt_cancellation(
        &mut self,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<PromptCancellation, DaemonError> {
        self.kernel_agents().finalize_active_prompt_cancellation(
            session_id,
            agent_id,
            provider_run_id,
        )
    }
}
