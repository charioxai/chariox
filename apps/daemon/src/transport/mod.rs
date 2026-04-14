use crate::app::provider_output::{
    pump_terminal_output_for_attachment, ProviderOutputPump, ProviderOutputPumpRequest,
};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptAttachment, PromptCancellation, PromptCompletion, PromptQueueItem};
use crate::terminal::TerminalOutputRecord;
use arroba_relay::protocol::ClientTarget;

use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

pub(crate) mod flow_control;
pub(crate) mod mcp_server;
pub(crate) mod relay_client;
pub(crate) mod relay_crypto;
pub(crate) mod relay_discovery;
pub(crate) mod relay_peer;
pub(crate) mod runtime_tools;

pub struct TransportService;

impl TransportService {
    pub fn schedule_direct_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<crate::session::PromptSubmissionOutcome, DaemonError> {
        app.submit_prompt(session_id, attachment_id, None, prompt, attachments)
    }

    pub fn schedule_direct_prompt_to_agent(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<crate::session::PromptSubmissionOutcome, DaemonError> {
        app.submit_prompt(
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            attachments,
        )
    }

    pub fn complete_active_prompt(
        app: &mut DaemonApp,
        session_id: &str,
    ) -> Result<PromptCompletion, DaemonError> {
        let agent_id = app
            .sessions()
            .get_session(session_id)?
            .active_prompt_agent_id()
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let provider_run_id = app
            .providers()
            .get_run_for_agent(session_id, &agent_id)
            .map(|run| run.id().to_string());
        app.complete_active_prompt(session_id, &agent_id, provider_run_id.as_deref())
    }

    pub fn cancel_active_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        app.cancel_active_prompt(session_id, attachment_id)
    }

    pub fn cancel_active_prompt_for_runtime(
        app: &mut DaemonApp,
        session_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        app.cancel_active_prompt_for_runtime(session_id)
    }

    pub fn pump_active_prompts(app: &mut DaemonApp) {
        app.pump_active_prompt_outputs();
        app.pump_workflow_watchdogs();
    }

    pub fn pump_terminal_output(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        pump_terminal_output_for_attachment(app, session_id, attachment_id)
    }

    pub fn pump_provider_output(
        app: &mut DaemonApp,
        session_id: &str,
        provider_run_id: &str,
        recipient_attachment_ids: Vec<String>,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        ProviderOutputPump::new(app).pump_provider_output(ProviderOutputPumpRequest {
            session_id,
            provider_run_id,
            recipient_attachment_ids,
        })
    }

    pub fn dispatch_workflow_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        target_agent_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
        let target_agent = app.agents().get_agent(target_agent_id)?;
        if let Some(remote_execution) = target_agent.remote_execution().cloned() {
            let workflow_context =
                app.remote_workflow_turn_context_for_prompt(session_id, target_agent_id, prompt)?;
            let response = app.block_on_relay_future(send_peer_request_via_temporary_connection(
                app.config(),
                ClientTarget {
                    daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::SubmitLeasedPrompt {
                    leased_agent_id: remote_execution.leased_agent_id,
                    prompt: prompt.prompt().to_string(),
                    attachments: app.serialize_remote_prompt_attachments(prompt.attachments())?,
                    workflow_context: Some(workflow_context),
                },
            ));
            return match response {
                Ok(RelayPeerResponse::LeasedPromptSubmitted { .. }) => Ok(()),
                Ok(other) => Err(DaemonError::LocalTransport {
                    operation: "dispatch remote workflow prompt",
                    message: format!("unexpected remote workflow prompt response: {other:?}"),
                }),
                Err(error) => Err(error),
            };
        }
        let dispatch = |app: &mut DaemonApp, provider_run_id: &str| {
            app.dispatch_prompt_to_provider(
                session_id,
                provider_run_id,
                prompt.source_attachment_id(),
                prompt.prompt(),
                prompt.attachments(),
            )
        };
        let mut last_retryable_error = None;
        for attempt in 0..3 {
            let provider_run_id =
                app.ensure_workflow_provider_run_from_runtime(session_id, target_agent_id)?;
            match dispatch(app, &provider_run_id) {
                Ok(()) => {
                    flow_control::note_prompt_started(app, &provider_run_id);
                    return Ok(());
                }
                Err(
                    error @ (DaemonError::InvalidProviderRunState { .. }
                    | DaemonError::NoActiveProviderRun { .. }
                    | DaemonError::PtyWrite { .. }
                    | DaemonError::PtyProcessNotFound { .. }),
                ) if attempt < 2 => {
                    last_retryable_error = Some(error);
                    continue;
                }
                Err(other) => return Err(other),
            }
        }
        Err(
            last_retryable_error.unwrap_or(DaemonError::NoActiveProviderRun {
                session_id: session_id.to_string(),
            }),
        )
    }

    pub fn cancel_active_prompt_after_dispatch_failure(
        app: &mut DaemonApp,
        session_id: &str,
        agent_id: &str,
        provider_run_id: Option<&str>,
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        match app
            .sessions_mut()
            .cancel_active_prompt(session_id, agent_id)
        {
            Ok((_, cancelled)) => {
                if let Some(provider_run_id) = provider_run_id {
                    flow_control::clear_prompt_activity(app, provider_run_id);
                }
                Ok(Some(cancelled))
            }
            Err(_) => Ok(None),
        }
    }
}
