use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptAttachment, PromptCancellation, PromptCompletion};

pub(crate) mod flow_control;

pub struct TransportService;

impl TransportService {
    pub fn schedule_direct_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<crate::session::PromptSubmissionOutcome, DaemonError> {
        app.submit_prompt(session_id, attachment_id, prompt, attachments)
    }

    pub fn complete_active_prompt(
        app: &mut DaemonApp,
        session_id: &str,
    ) -> Result<PromptCompletion, DaemonError> {
        app.complete_active_prompt(session_id)
    }

    pub fn cancel_active_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        app.cancel_active_prompt(session_id, attachment_id)
    }

    pub fn pump_active_prompts(app: &mut DaemonApp) {
        app.pump_active_prompt_outputs();
    }
}
