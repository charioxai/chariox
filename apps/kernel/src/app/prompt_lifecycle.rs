use crate::agent::RemoteAgentBinding;
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::execution_lease::RemoteWorkflowTurnContext;
use crate::provider::ProviderRunState;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptOrigin, PromptQueueItem,
    PromptSubmissionOutcome,
};
use crate::transport::relay_peer::RelayPromptAttachment;
use base64::Engine;
use std::fs;

pub(crate) struct RemoteWorkflowTurnContextResolver<'a> {
    app: &'a DaemonApp,
}

impl<'a> RemoteWorkflowTurnContextResolver<'a> {
    pub(crate) fn new(app: &'a DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn remote_workflow_turn_context_for_prompt(
        &self,
        session_id: &str,
        target_agent_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<RemoteWorkflowTurnContext, DaemonError> {
        let workflow_run_id =
            prompt
                .workflow_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow run id".to_string(),
                })?;
        let workflow_node_run_id =
            prompt
                .workflow_node_run_id()
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: "remote workflow prompt is missing workflow node run id".to_string(),
                })?;
        let workflow_run = self
            .app
            .sessions()
            .resolve_workflow_run_ref(session_id, workflow_run_id)?;
        let delivery_token = workflow_run
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .map(|envelope| envelope.delivery_token().to_string())
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "dispatch remote workflow prompt",
                message: format!(
                    "workflow node run `{workflow_node_run_id}` has no prepared turn envelope"
                ),
            })?;
        Ok(RemoteWorkflowTurnContext {
            home_kernel_id: self.app.config().daemon_id.clone(),
            home_session_id: session_id.to_string(),
            home_agent_id: target_agent_id.to_string(),
            workflow_run_id: workflow_run.id().to_string(),
            workflow_node_run_id: workflow_node_run_id.to_string(),
            delivery_token,
        })
    }
}

pub(crate) struct ProviderPromptDispatcher<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> ProviderPromptDispatcher<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn dispatch_prompt_to_provider(
        &mut self,
        session_id: &str,
        provider_run_id: &str,
        prompt_id: &str,
        attachment_id: &str,
        prompt: &str,
        hidden_system_context: &str,
        attachments: &[PromptAttachment],
    ) -> Result<(), DaemonError> {
        let agent_id = self
            .app
            .providers
            .get_run(provider_run_id)?
            .agent_instance_id()
            .unwrap_or_default()
            .to_string();
        if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
            .as_ref()
            .is_none_or(|active| active.id() != prompt_id)
        {
            return Ok(());
        }
        let provider_run = crate::app::ProviderRunReadService::new(self.app)
            .ensure_provider_run_in_session(session_id, provider_run_id)?;
        if provider_run.state() != ProviderRunState::Running {
            return Err(DaemonError::InvalidProviderRunState {
                provider_run_id: provider_run_id.to_string(),
                state: provider_run.state(),
                operation: "submit prompt",
            });
        }
        let agent_id = provider_run
            .agent_instance_id()
            .ok_or_else(|| DaemonError::AgentNotFound {
                agent_id: "provider run has no agent".to_string(),
            })?
            .to_string();
        self.app.mark_active_prompt_delivery(
            session_id,
            &agent_id,
            prompt_id,
            crate::session::DurablePromptDeliveryPhase::Dispatching,
            Some(provider_run_id.to_string()),
            provider_run.provider_session_id().map(str::to_string),
        )?;
        let recipients = self.app.attachments.list_session_attachment_ids(session_id);
        let _ = crate::app::provider_output::ProviderOutputPump::new(self.app)
            .pump_provider_output(crate::app::provider_output::ProviderOutputPumpRequest {
                session_id,
                provider_run_id,
                recipient_attachment_ids: recipients,
                initial_liveness_already_checked: false,
            })?;
        if self
            .app
            .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
            .as_ref()
            .is_none_or(|active| active.id() != prompt_id)
        {
            return Ok(());
        }

        if self
            .app
            .providers
            .run_uses_structured_prompt_io(&provider_run)
        {
            let source_client_id = self
                .app
                .prompt_owner_active_prompt_for_agent(session_id, &agent_id)?
                .and_then(|active| {
                    self.app
                        .attachments
                        .get_attachment(active.source_attachment_id())
                        .ok()
                })
                .map(|attachment| attachment.client_id().to_string());
            let mode = crate::prompt_assembly::provider_turn_mode_for_prompt(
                &agent_id,
                self.app.agents.get_agent(&agent_id)?.is_metaagent(),
                source_client_id.as_deref(),
                hidden_system_context,
            );
            self.app.providers.enqueue_structured_prompt_submit(
                session_id.to_string(),
                provider_run_id.to_string(),
                agent_id,
                prompt_id.to_string(),
                &provider_run,
                prompt,
                hidden_system_context,
                attachments,
                mode,
                false,
            )?;
            return Ok(());
        }

        let provider_prompt = join_hidden_context(hidden_system_context, prompt);
        if crate::provider::provider_run_uses_claude_native_bridge(&provider_run) {
            self.app.terminal.record_input(
                session_id,
                provider_run_id,
                attachment_id,
                provider_prompt.as_bytes(),
            );
            self.app.process_claude_native_bridge_for_runtime(
                session_id,
                provider_run_id,
                &provider_run,
            )?;
            self.app.mark_active_prompt_delivery(
                session_id,
                &agent_id,
                prompt_id,
                crate::session::DurablePromptDeliveryPhase::Delivered,
                Some(provider_run_id.to_string()),
                provider_run.provider_session_id().map(str::to_string),
            )?;
            return Ok(());
        }
        crate::app::terminal_input::ProviderTerminalInput::new(self.app).send_provider_input(
            session_id,
            provider_run_id,
            attachment_id,
            &crate::app::terminal_input::provider_prompt_input(&provider_prompt),
        )?;
        self.app.mark_active_prompt_delivery(
            session_id,
            &agent_id,
            prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id.to_string()),
            provider_run.provider_session_id().map(str::to_string),
        )?;
        Ok(())
    }
}

fn join_hidden_context(first: &str, second: &str) -> String {
    match (first.trim(), second.trim()) {
        ("", "") => String::new(),
        (first, "") => first.to_string(),
        ("", second) => second.to_string(),
        (first, second) => format!("{first}\n\n{second}"),
    }
}

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
    pub(crate) refresh_projection: bool,
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
    pub(crate) prompt_id: String,
    pub(crate) target_active_prompt_id: Option<String>,
    pub(crate) source_attachment_id: String,
    pub(crate) prompt: String,
    pub(crate) hidden_system_context: String,
    pub(crate) attachments: Vec<PromptAttachment>,
    pub(crate) prompt_origin: PromptOrigin,
    pub(crate) external_provider: Option<String>,
    pub(crate) external_provider_session_id: Option<String>,
    pub(crate) external_provider_turn_id: Option<String>,
    pub(crate) steering: bool,
}

pub(crate) struct KernelRemotePromptDispatch {
    pub(crate) session_id: String,
    pub(crate) agent_id: String,
    pub(crate) prompt_id: String,
    pub(crate) worker_kernel_id: String,
    pub(crate) leased_agent_id: String,
    pub(crate) relay_url: Option<String>,
    pub(crate) relay_token: Option<String>,
    pub(crate) source_attachment_id: String,
    pub(crate) prompt: String,
    pub(crate) attachments: Vec<PromptAttachment>,
    pub(crate) workspace_live_sync_mode: Option<crate::config::WorkspaceLiveSyncMode>,
    pub(crate) prompt_origin: PromptOrigin,
    pub(crate) external_provider: Option<String>,
    pub(crate) external_provider_session_id: Option<String>,
    pub(crate) external_provider_turn_id: Option<String>,
    pub(crate) workflow_context: Option<RemoteWorkflowTurnContext>,
}

pub(crate) struct KernelPromptCancellation {
    pub(crate) cancellation: PromptCancellation,
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) dispatch: Option<KernelPromptAbortDispatch>,
}

pub(crate) struct KernelQueuedPromptSteer {
    pub(crate) prompt: PromptQueueItem,
    pub(crate) session: crate::session::RuntimeSession,
    pub(crate) dispatch: Option<KernelPromptDispatch>,
}

pub(crate) struct KernelQueuedPromptCancellation {
    pub(crate) prompt: PromptQueueItem,
    pub(crate) session: crate::session::RuntimeSession,
}

pub(crate) struct KernelQueuedPromptUpdate {
    pub(crate) prompt: PromptQueueItem,
    pub(crate) session: crate::session::RuntimeSession,
}

#[derive(Clone)]
pub(crate) struct KernelPromptAbortDispatch {
    pub(crate) session_id: String,
    pub(crate) provider_run_id: String,
    pub(crate) source_attachment_id: String,
}

impl DaemonApp {
    #[doc(hidden)]
    pub fn submit_prompt(
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

    pub(crate) fn record_native_prompt_started_with_attachments(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        history_source_attachment_id: &str,
        target_agent_id: &str,
        prompt: &str,
        attachments: Vec<crate::session::PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        crate::app::KernelAgentService::new(self).record_native_prompt_started(
            session_id,
            attachment_id,
            history_source_attachment_id,
            target_agent_id,
            prompt,
            attachments,
        )
    }

    pub(crate) fn finish_kernel_prompt_dispatch(
        &mut self,
        session_id: String,
        provider_run_id: String,
        agent_id: String,
        prompt_id: String,
        result: Result<crate::provider::ProviderPromptSubmitAcknowledgement, DaemonError>,
    ) -> Result<(), DaemonError> {
        let acknowledgement = match result {
            Ok(acknowledgement) => acknowledgement,
            Err(error) => {
                crate::app::KernelAgentService::new(self).cancel_active_after_prompt_start_failure(
                    &session_id,
                    &agent_id,
                    &provider_run_id,
                );
                let _ =
                    crate::app::KernelSessionReadService::new(self).session_snapshot(&session_id);
                self.record_notice(
                    &session_id,
                    Some(&provider_run_id),
                    self.attachments.list_session_attachment_ids(&session_id),
                    format!("Prompt dispatch failed after acknowledgement: {error}"),
                );
                return Err(error);
            }
        };
        let run = self
            .providers
            .apply_prompt_submit_acknowledgement(&provider_run_id, &acknowledgement)?;
        let agent = self.agents.set_agent_runtime_profile_with_account_profile(
            &agent_id,
            run.provider(),
            Some(run.model().to_string()),
            run.variant().map(str::to_string),
            Some(run.account_profile().to_string()),
            run.resume_state().clone(),
        )?;
        self.durable_state_store().append_event(
            "agent.runtime_profile_updated",
            Some(agent.id().to_string()),
            serde_json::json!({
                "agent": &agent,
                "provider_run_id": run.id(),
                "reason": "prompt_delivery_acknowledged",
            }),
        )?;
        self.update_provider_run_projection(run.clone());
        self.mark_active_prompt_delivery(
            &session_id,
            &agent_id,
            &prompt_id,
            crate::session::DurablePromptDeliveryPhase::Delivered,
            Some(provider_run_id),
            run.provider_session_id().map(str::to_string),
        )?;
        let active = self.prompt_owner_active_prompt_for_agent(&session_id, &agent_id)?;
        if let Some(active) = active {
            if active.id() == prompt_id
                && active.durable_recovery_phase()
                    == Some(crate::session::DurablePromptDeliveryPhase::Dispatching)
            {
                if let Some(operation_id) = active.durable_recovery_operation_id() {
                    let session = self.sessions.get_session(&session_id)?;
                    let prompt = self.prompt_state_owner.mark_active_prompt_recovery_phase(
                        &session,
                        &agent_id,
                        &prompt_id,
                        operation_id,
                        crate::session::DurablePromptDeliveryPhase::Delivered,
                    )?;
                    let _ = prompt;
                    self.mirror_prompt_owner_agent_state(&session_id, &agent_id)?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn finish_kernel_remote_prompt_dispatch(
        &mut self,
        dispatch: KernelRemotePromptDispatch,
        result: Result<String, DaemonError>,
    ) -> Result<(), DaemonError> {
        match result {
            Ok(remote_provider_run_id) => {
                let _ = self
                    .agents
                    .set_remote_execution_active_worker_provider_run_id(
                        &dispatch.agent_id,
                        Some(remote_provider_run_id.clone()),
                    )?;
                self.echo_prompt_to_other_attachments(
                    &dispatch.session_id,
                    &remote_provider_run_id,
                    &dispatch.prompt_id,
                    &dispatch.source_attachment_id,
                    &dispatch.prompt,
                    &dispatch.attachments,
                );
                self.mark_active_prompt_delivery(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                    &dispatch.prompt_id,
                    crate::session::DurablePromptDeliveryPhase::Delivered,
                    Some(remote_provider_run_id),
                    None,
                )?;
                Ok(())
            }
            Err(error) => {
                let _ = self
                    .agents
                    .set_remote_execution_active_worker_provider_run_id(&dispatch.agent_id, None);
                let _ = self.prompt_owner_cancel_active_prompt_only(
                    &dispatch.session_id,
                    &dispatch.agent_id,
                );
                let _ = crate::app::KernelSessionReadService::new(self)
                    .session_snapshot(&dispatch.session_id);
                self.record_notice_for_agent(
                    &dispatch.session_id,
                    None,
                    Some(&dispatch.agent_id),
                    self.attachments
                        .list_session_attachment_ids(&dispatch.session_id),
                    format!("Remote prompt dispatch failed after acknowledgement: {error}"),
                );
                Err(error)
            }
        }
    }

    #[doc(hidden)]
    pub fn complete_active_prompt(
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

    #[doc(hidden)]
    pub fn cancel_active_prompt(
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
            if let Ok(provider_run) = self.providers.get_run(&provider_run_id) {
                if let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) {
                    let _ = self.finalize_active_prompt_cancellation(
                        &session_id,
                        &agent_id,
                        Some(&provider_run_id),
                    );
                }
            }
            return Err(error);
        }
        if let Ok(provider_run) = self.providers.get_run(&provider_run_id) {
            if let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) {
                let _ = self.finalize_active_prompt_cancellation(
                    &session_id,
                    &agent_id,
                    Some(&provider_run_id),
                );
            }
        }
        Ok(())
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
                finished.prompt_id,
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
        relay_url: Option<&str>,
        relay_token: Option<&str>,
    ) -> Result<Option<crate::session::PromptQueueItem>, DaemonError> {
        crate::app::KernelAgentService::new(self).advance_next_queued_prompt_remote(
            session_id,
            agent_id,
            worker_kernel_id,
            leased_agent_id,
            relay_url,
            relay_token,
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
