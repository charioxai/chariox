use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::session::{PromptQueueItem, PromptSubmissionOutcome, WorkflowCompletionUpdate};
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
                    let _ = on_workflow_prompt_cancelled(app, session_id, &cancelled);
                }
                return Err(error);
            }
            on_workflow_prompt_started(app, session_id, &prompt)?;
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

pub fn is_workflow_prompt_attachment(attachment_id: &str) -> bool {
    DaemonApp::is_workflow_prompt_source_attachment_id(attachment_id)
}

pub fn ensure_workflow_provider_run_for_agent(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Result<String, DaemonError> {
    app.ensure_workflow_provider_run_for_agent(session_id, agent_id)
}

pub fn on_workflow_prompt_started(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    let workflow_run = app.sessions_mut().start_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    )?;
    app.record_notice(
        session_id,
        app.sessions()
            .get_session(session_id)?
            .active_provider_run_id(),
        app.attachments().list_session_attachment_ids(session_id),
        format!(
            "Workflow run `{}` started on agent `{}`.",
            workflow_run.id(),
            prompt.target_agent_id()
        ),
    );
    Ok(())
}

pub fn on_workflow_prompt_completed(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    let completion_snapshot = app.build_workflow_completion_snapshot(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        provider_run_id,
    );
    let max_turns = app.workflow_max_turns(session_id);
    let WorkflowCompletionUpdate {
        workflow_run,
        dispatches,
        validation_warnings,
    } = app.sessions_mut().complete_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        completion_snapshot,
        max_turns,
    )?;
    if !validation_warnings.is_empty() {
        app.write_workflow_control_mailbox(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            &validation_warnings,
        );
        for warning in &validation_warnings {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow output validation warning on edge `{}`: {}",
                    warning.edge_id, warning.message
                ),
            );
        }
    }
    app.schedule_workflow_dispatches(session_id, workflow_run.id(), &dispatches);
    let state_suffix = match workflow_run.status() {
        crate::session::WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
        crate::session::WorkflowRunStatus::Completed => "completed",
        crate::session::WorkflowRunStatus::Stopped => "stopped after reaching the max turn limit",
        _ => "updated",
    };
    app.record_notice(
        session_id,
        None,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{}` {state_suffix}.", workflow_run.id()),
    );
    Ok(())
}

pub fn on_workflow_prompt_cancelled(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    let workflow_run = app.sessions_mut().stop_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    )?;
    app.record_notice(
        session_id,
        None,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{}` was stopped.", workflow_run.id()),
    );
    Ok(())
}
