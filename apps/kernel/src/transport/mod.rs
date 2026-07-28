pub(crate) mod event_delivery_client;
pub(crate) mod flow_control;
pub(crate) mod kernel_protocol;
pub(crate) mod mcp_server;
pub(crate) mod relay_client;
pub(crate) mod relay_crypto;
pub(crate) mod relay_discovery;
pub(crate) mod relay_peer;
pub(crate) mod runtime_tools;

use crate::app::provider_output::{ProviderOutputPump, ProviderOutputPumpRequest};
use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{
    PromptAttachment, PromptCancellation, PromptCompletion, PromptSubmissionOutcome,
};
use crate::terminal::TerminalOutputRecord;

#[doc(hidden)]
pub struct TransportService;

impl TransportService {
    #[doc(hidden)]
    pub fn schedule_direct_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        app.submit_prompt(session_id, attachment_id, None, prompt, attachments)
    }

    #[doc(hidden)]
    pub fn complete_active_prompt(
        app: &mut DaemonApp,
        session_id: &str,
    ) -> Result<PromptCompletion, DaemonError> {
        let agent_id = app
            .prompt_owner_active_prompt_agent_id(session_id)?
            .ok_or_else(|| DaemonError::NoActivePrompt {
                session_id: session_id.to_string(),
            })?;
        let provider_run_id = app
            .sessions()
            .get_session(session_id)?
            .active_provider_run_id()
            .map(str::to_string);
        app.complete_active_prompt(session_id, &agent_id, provider_run_id.as_deref())
    }

    #[doc(hidden)]
    pub fn cancel_active_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        app.cancel_active_prompt(session_id, attachment_id)
    }

    #[doc(hidden)]
    pub fn pump_terminal_output(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Vec<TerminalOutputRecord>, DaemonError> {
        crate::app::provider_output::pump_terminal_output_for_attachment(
            app,
            session_id,
            attachment_id,
        )
    }

    #[doc(hidden)]
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
            initial_liveness_already_checked: false,
        })
    }
}
