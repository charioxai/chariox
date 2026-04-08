use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptAttachment, PromptCancellation, PromptCompletion, PromptQueueItem};

pub(crate) mod flow_control;
pub(crate) mod mcp_server;
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

    pub fn dispatch_workflow_prompt(
        app: &mut DaemonApp,
        session_id: &str,
        target_agent_id: &str,
        prompt: &PromptQueueItem,
    ) -> Result<(), DaemonError> {
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
                crate::scheduler::runtime::ensure_workflow_provider_run_for_agent(
                    app,
                    session_id,
                    target_agent_id,
                )?;
            match dispatch(app, &provider_run_id) {
                Ok(()) => {
                    flow_control::note_prompt_started(app, session_id);
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
    ) -> Result<Option<PromptQueueItem>, DaemonError> {
        match app.sessions_mut().cancel_active_prompt(session_id) {
            Ok((_, cancelled)) => {
                flow_control::clear_prompt_activity(app, session_id);
                Ok(Some(cancelled))
            }
            Err(_) => Ok(None),
        }
    }
}
