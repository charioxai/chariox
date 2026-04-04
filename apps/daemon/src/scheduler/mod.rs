use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptAttachment, PromptCancellation, PromptCompletion};

pub struct SchedulerService;

impl SchedulerService {
    pub fn schedule_workflow_node_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_run_id: &str,
        workflow_node_run_id: &str,
        target_agent_id: &str,
        node_id: &str,
        prompt: &str,
    ) -> Result<(), DaemonError> {
        app.schedule_workflow_node_prompt(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            target_agent_id,
            node_id,
            prompt,
        )
    }

    pub fn schedule_direct_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        attachment_id: &str,
        prompt: &str,
        attachments: Vec<PromptAttachment>,
    ) -> Result<crate::session::PromptSubmissionOutcome, DaemonError> {
        app.submit_prompt(session_id, attachment_id, prompt, attachments)
    }

    pub fn pump_active_prompts(app: &mut DaemonApp) {
        app.pump_active_prompt_outputs();
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
}
