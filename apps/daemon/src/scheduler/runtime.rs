use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::PromptSubmissionOutcome;
use crate::transport::TransportService;

pub fn schedule_workflow_node_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    node_id: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let (_session, outcome) = app.sessions_mut().submit_workflow_prompt(
        session_id,
        &DaemonApp::workflow_prompt_source_attachment_id(workflow_run_id),
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
        prompt.to_string(),
    )?;

    match outcome {
        PromptSubmissionOutcome::Started { prompt } => {
            if let Err(error) = TransportService::dispatch_workflow_prompt(
                app,
                session_id,
                target_agent_id,
                &prompt,
            ) {
                if let Ok(Some(cancelled)) =
                    TransportService::cancel_active_prompt_after_dispatch_failure(app, session_id)
                {
                    let _ = app.reconcile_workflow_prompt_cancelled(session_id, &cancelled);
                }
                return Err(error);
            }
            app.reconcile_workflow_prompt_started(session_id, &prompt)?;
        }
        PromptSubmissionOutcome::Queued { .. } => {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` queued node `{node_id}` behind the current active prompt."
                ),
            );
        }
    }

    Ok(())
}
