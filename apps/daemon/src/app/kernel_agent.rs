use super::prompt_lifecycle::{KernelPromptCancellation, KernelPromptSubmission};
use super::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptAttachment, PromptCancellation, PromptSubmissionOutcome};

pub(crate) struct KernelAgentService<'a> {
    app: &'a mut DaemonApp,
}

impl<'a> KernelAgentService<'a> {
    pub(crate) fn new(app: &'a mut DaemonApp) -> Self {
        Self { app }
    }

    pub(crate) fn submit_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<PromptSubmissionOutcome, DaemonError> {
        self.app.submit_prompt(
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            attachments,
        )
    }

    pub(crate) fn submit_prompt_for_kernel(
        &mut self,
        session_id: &str,
        attachment_id: &str,
        target_agent_id: Option<&str>,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<KernelPromptSubmission, DaemonError> {
        self.app.submit_prompt_for_kernel(
            session_id,
            attachment_id,
            target_agent_id,
            prompt,
            attachments,
        )
    }

    pub(crate) fn cancel_active_prompt(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<PromptCancellation, DaemonError> {
        self.app.cancel_active_prompt(session_id, attachment_id)
    }

    pub(crate) fn cancel_active_prompt_for_kernel(
        &mut self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<KernelPromptCancellation, DaemonError> {
        self.app
            .cancel_active_prompt_for_kernel(session_id, attachment_id)
    }
}
