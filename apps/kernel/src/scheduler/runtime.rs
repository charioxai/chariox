use std::collections::BTreeSet;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::{ControlOperation, RuntimeProviderRun};
use crate::session::{
    PromptQueueItem, PromptSubmissionOutcome, WorkflowCompletionUpdate, WorkflowConsole,
    WorkflowConsoleEntry, WorkflowDefinition, WorkflowDispatch, WorkflowFailureEvent,
    WorkflowFailureKind, WorkflowMessage, WorkflowNodeRunStatus, WorkflowRun, WorkflowRunStatus,
};

const WORKFLOW_PROMPT_SOURCE_PREFIX: &str = "workflow-run:";
const WORKFLOW_MAX_TURNS_CONFIG_KEY: &str = "workflow.max_turns";

mod completion;
mod control_mailbox;
mod failures;
mod lifecycle;
mod prompt_dispatch;
mod workflow_validation;

pub use lifecycle::{
    on_workflow_prompt_cancelled, on_workflow_prompt_completed, on_workflow_prompt_started,
    on_workflow_provider_failure,
};
pub use workflow_validation::validate_workflow_agents;

use completion::build_workflow_completion_snapshot;
pub(crate) use completion::build_workflow_completion_snapshot_from_history;
use control_mailbox::{clear_workflow_control_mailbox, workflow_node_control_contents};
use failures::{provider_run_terminal_diagnostic, record_and_route_workflow_failure};
use prompt_dispatch::{dispatch_workflow_prompt, submit_claimed_workflow_prompt};

pub fn schedule_workflow_run_entry_node(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run: &WorkflowRun,
) -> Result<(), DaemonError> {
    let endpoint_prompt = workflow_run
        .invocation_prompt()
        .map(str::trim)
        .unwrap_or("");
    let node_run = workflow_run.node_runs().first().ok_or_else(|| {
        DaemonError::InvalidWorkflowGraphReference {
            session_id: session_id.to_string(),
            workflow_id: workflow_run.workflow_id().to_string(),
            reference: workflow_run.id().to_string(),
            message: "workflow run has no entry node run",
        }
    })?;
    schedule_workflow_node_prompt(
        app,
        session_id,
        workflow_run.id(),
        node_run.id(),
        node_run.agent_id(),
        node_run.node_id(),
        &prepare_workflow_turn_prompt(
            app,
            session_id,
            workflow_run.id(),
            node_run.id(),
            node_run.node_id(),
            endpoint_prompt,
            None,
        )?,
    )
}

pub fn schedule_workflow_dispatches(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    dispatches: &[WorkflowDispatch],
) {
    for dispatch in dispatches {
        app.record_notice(
            session_id,
            None,
            app.attachments().list_session_attachment_ids(session_id),
            format!(
                "Workflow run `{workflow_run_id}` routed {} upstream message(s) to node `{}`.",
                dispatch.messages.len(),
                dispatch.node_run.node_id()
            ),
        );
        let prompt = match prepare_workflow_turn_prompt(
            app,
            session_id,
            workflow_run_id,
            dispatch.node_run.id(),
            dispatch.node_run.node_id(),
            dispatch.endpoint_prompt.as_deref().unwrap_or(""),
            Some(&dispatch.messages),
        ) {
            Ok(prompt) => prompt,
            Err(error) => {
                app.record_notice(
                    session_id,
                    None,
                    app.attachments().list_session_attachment_ids(session_id),
                    format!(
                        "Workflow run `{workflow_run_id}` could not prepare downstream node `{}`: {}",
                        dispatch.node_run.node_id(),
                        error
                    ),
                );
                continue;
            }
        };
        if let Err(error) = schedule_workflow_node_prompt(
            app,
            session_id,
            workflow_run_id,
            dispatch.node_run.id(),
            dispatch.node_run.agent_id(),
            dispatch.node_run.node_id(),
            &prompt,
        ) {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` could not schedule downstream node `{}`: {}",
                    dispatch.node_run.node_id(),
                    error
                ),
            );
            fail_workflow_node_after_schedule_error(
                app,
                session_id,
                workflow_run_id,
                dispatch.node_run.id(),
                &error,
            );
        }
    }
}

pub fn schedule_workflow_node_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    node_id: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let _ = app
        .sessions_mut()
        .set_focused_agent(session_id, Some(target_agent_id.to_string()));
    let delivery_token = workflow_turn_delivery_token(workflow_node_run_id);
    let mailbox_content = workflow_node_control_contents(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
    );
    let handoff_payloads_json =
        crate::scheduler::prompt_injection::workflow_handoff_payloads_from_prompt(prompt);
    app.sessions_mut().prepare_workflow_turn(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        delivery_token,
        prompt.to_string(),
        mailbox_content,
        handoff_payloads_json,
    )?;
    dispatch_prepared_workflow_node_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        target_agent_id,
        node_id,
        prompt,
    )
}

fn dispatch_prepared_workflow_node_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    node_id: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let provider_run_id =
        match ensure_workflow_provider_run_for_agent(app, session_id, target_agent_id) {
            Ok(provider_run_id) => provider_run_id,
            Err(error) => return Err(error),
        };
    match app.acquire_workflow_node_workspace_claim(
        session_id,
        &provider_run_id,
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
    ) {
        Ok(()) => {
            let _ = app
                .sessions_mut()
                .ready_workflow_node_after_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
        }
        Err(error @ DaemonError::WorkspaceClaimConflict { .. }) => {
            let _ = app.sessions_mut().block_workflow_node_on_workspace_claim(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            );
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` blocked node `{node_id}` on a workspace claim: {error}"
                ),
            );
            let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id);
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    let outcome = match submit_claimed_workflow_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        target_agent_id,
        prompt,
    ) {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = app.release_prompt_workspace_claim(&provider_run_id);
            return Err(error);
        }
    };
    handle_workflow_prompt_submission_outcome(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        target_agent_id,
        node_id,
        outcome,
    )
}

pub fn retry_blocked_workflow_claims(app: &mut DaemonApp) -> BTreeSet<String> {
    let mut blocked = Vec::new();
    for session in app.sessions().list_sessions() {
        let session_id = session.id().to_string();
        for workflow_run in session.workflow_runs() {
            for node_run in workflow_run.node_runs() {
                if node_run.status() != WorkflowNodeRunStatus::BlockedOnWorkspaceClaim {
                    continue;
                }
                let Some(prompt) = node_run
                    .turn_envelope()
                    .and_then(|envelope| envelope.rendered_prompt())
                    .map(str::to_string)
                else {
                    continue;
                };
                blocked.push((
                    session_id.clone(),
                    workflow_run.id().to_string(),
                    node_run.id().to_string(),
                    node_run.agent_id().to_string(),
                    node_run.node_id().to_string(),
                    prompt,
                ));
            }
        }
    }

    let mut affected_sessions = BTreeSet::new();
    for (session_id, workflow_run_id, workflow_node_run_id, agent_id, node_id, prompt) in blocked {
        if let Err(error) = retry_prepared_workflow_node_prompt(
            app,
            &session_id,
            &workflow_run_id,
            &workflow_node_run_id,
            &agent_id,
            &node_id,
            &prompt,
        ) {
            app.record_notice(
                &session_id,
                None,
                app.attachments().list_session_attachment_ids(&session_id),
                format!(
                    "Workflow run `{workflow_run_id}` could not retry blocked node `{node_id}`: {error}"
                ),
            );
        }
        affected_sessions.insert(session_id);
    }
    affected_sessions
}

fn retry_prepared_workflow_node_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    node_id: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let provider_run_id = ensure_workflow_provider_run_for_agent(app, session_id, target_agent_id)?;
    match app.acquire_workflow_node_workspace_claim(
        session_id,
        &provider_run_id,
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
    ) {
        Ok(()) => {
            let _ = app
                .sessions_mut()
                .ready_workflow_node_after_workspace_claim(
                    session_id,
                    workflow_run_id,
                    workflow_node_run_id,
                );
        }
        Err(DaemonError::WorkspaceClaimConflict { .. }) => return Ok(()),
        Err(error) => return Err(error),
    }
    let outcome = submit_claimed_workflow_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        target_agent_id,
        prompt,
    )?;
    handle_workflow_prompt_submission_outcome(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        target_agent_id,
        node_id,
        outcome,
    )
}

pub fn resume_workflow_run(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_ref: &str,
) -> Result<WorkflowRun, DaemonError> {
    let workflow_run = app
        .sessions_mut()
        .resume_workflow_run(session_id, workflow_run_ref)?;
    let resumable_node_runs = workflow_run
        .node_runs()
        .iter()
        .filter(|node_run| {
            node_run.status() == WorkflowNodeRunStatus::Ready
                && node_run
                    .turn_envelope()
                    .and_then(|envelope| envelope.rendered_prompt())
                    .is_some()
        })
        .map(|node_run| {
            (
                node_run.id().to_string(),
                node_run.node_id().to_string(),
                node_run.agent_id().to_string(),
                node_run
                    .turn_envelope()
                    .and_then(|envelope| envelope.rendered_prompt())
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    for (workflow_node_run_id, node_id, agent_id, prompt) in resumable_node_runs {
        resume_existing_workflow_node_prompt(
            app,
            session_id,
            workflow_run.id(),
            &workflow_node_run_id,
            &node_id,
            &agent_id,
            &prompt,
        )?;
    }
    app.sessions()
        .resolve_workflow_run_ref(session_id, workflow_run.id())
}

fn resume_existing_workflow_node_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
    target_agent_id: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let _ = app
        .sessions_mut()
        .set_focused_agent(session_id, Some(target_agent_id.to_string()));
    dispatch_prepared_workflow_node_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        target_agent_id,
        node_id,
        prompt,
    )
}

fn handle_workflow_prompt_submission_outcome(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    node_id: &str,
    outcome: PromptSubmissionOutcome,
) -> Result<(), DaemonError> {
    match outcome {
        PromptSubmissionOutcome::Started { prompt } => {
            if let Err(error) = dispatch_workflow_prompt(app, session_id, target_agent_id, &prompt)
            {
                record_and_route_workflow_failure(
                    app,
                    session_id,
                    workflow_run_id,
                    &WorkflowFailureEvent::new(
                        WorkflowFailureKind::TransportFailure,
                        workflow_node_run_id,
                        Vec::new(),
                        error.to_string(),
                    ),
                );
                let provider_run_id = app
                    .providers()
                    .get_run_for_agent(session_id, target_agent_id)
                    .map(|run| run.id().to_string());
                if let Ok(cancelled) =
                    app.prompt_owner_cancel_active_prompt_only(session_id, target_agent_id)
                {
                    if let Some(provider_run_id) = provider_run_id.as_deref() {
                        crate::transport::flow_control::clear_prompt_activity(app, provider_run_id);
                    }
                    let _ = on_workflow_prompt_cancelled(app, session_id, &cancelled);
                }
                return Err(error);
            }
            app.sessions_mut().mark_workflow_turn_dispatched(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
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

fn fail_workflow_node_after_schedule_error(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    error: &DaemonError,
) {
    record_and_route_workflow_failure(
        app,
        session_id,
        workflow_run_id,
        &WorkflowFailureEvent::new(
            WorkflowFailureKind::TransportFailure,
            workflow_node_run_id,
            Vec::new(),
            error.to_string(),
        ),
    );
    let _ = app.sessions_mut().fail_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    );
    match app.start_next_queued_workflow_prompt(session_id) {
        Ok(Some(crate::app::workflow_runtime::WorkflowLaunchOutcome::Started {
            workflow_run,
            workflow,
            endpoint,
        })) => {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Started queued workflow run `{}` for workflow `{}` endpoint `{}`.",
                    workflow_run.id(),
                    workflow.id(),
                    endpoint.id()
                ),
            );
        }
        Ok(Some(crate::app::workflow_runtime::WorkflowLaunchOutcome::Enqueued { .. })) => {}
        Ok(None) => {}
        Err(error) => {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!("Failed to start queued workflow prompt: {error}"),
            );
        }
    }
    let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id);
}

pub fn workflow_max_turns(app: &DaemonApp, session_id: &str) -> Option<usize> {
    let session = app.sessions().get_session(session_id).ok()?;
    session
        .config_state()
        .values()
        .get(WORKFLOW_MAX_TURNS_CONFIG_KEY)
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .or(Some(
            crate::session::DEFAULT_WORKFLOW_RUN_MAX_TURNS_SAFETY_LIMIT,
        ))
}

pub fn is_workflow_prompt_attachment(attachment_id: &str) -> bool {
    attachment_id.starts_with(WORKFLOW_PROMPT_SOURCE_PREFIX)
}

pub fn workflow_prompt_source_attachment_id(workflow_run_id: &str) -> String {
    format!("{WORKFLOW_PROMPT_SOURCE_PREFIX}{workflow_run_id}")
}

pub fn ensure_workflow_provider_run_for_agent(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
) -> Result<String, DaemonError> {
    ensure_workflow_provider_run_for_agent_with_event_reply(app, session_id, agent_id, false, false, false)
}

pub fn ensure_workflow_provider_run_for_agent_with_event_reply(
    app: &mut DaemonApp,
    session_id: &str,
    agent_id: &str,
    event_reply_enabled: bool,
    event_context_enabled: bool,
    event_actions_enabled: bool,
) -> Result<String, DaemonError> {
    prompt_dispatch::ensure_workflow_provider_run_for_agent(
        app,
        session_id,
        agent_id,
        event_reply_enabled,
        event_context_enabled,
        event_actions_enabled,
    )
}

pub fn read_workflow_console(
    app: &DaemonApp,
    session_id: &str,
    workflow_id: &str,
) -> Result<WorkflowConsole, DaemonError> {
    app.sessions()
        .read_workflow_console(session_id, workflow_id)
}

pub fn write_workflow_console(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_id: &str,
    workflow_node_run_id: &str,
    text: &str,
) -> Result<WorkflowConsoleEntry, DaemonError> {
    let source_agent_id = app
        .sessions()
        .get_session(session_id)
        .ok()
        .and_then(|session| {
            session
                .workflow_runs()
                .iter()
                .find(|run| run.workflow_id() == workflow_id)
                .and_then(|run| {
                    run.node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == workflow_node_run_id)
                        .map(|node_run| node_run.agent_id().to_string())
                })
        });
    app.sessions_mut().append_workflow_console_entry(
        session_id,
        workflow_id,
        Some(workflow_node_run_id.to_string()),
        source_agent_id,
        text,
    )
}

pub fn clear_workflow_console(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_id: &str,
) -> Result<WorkflowConsole, DaemonError> {
    app.sessions_mut()
        .clear_workflow_console(session_id, workflow_id)
}

fn prepare_workflow_turn_prompt(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
    endpoint_prompt: &str,
    handoff_messages: Option<&[WorkflowMessage]>,
) -> Result<String, DaemonError> {
    crate::scheduler::prompt_injection::render_workflow_turn_prompt_from_messages(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
        endpoint_prompt,
        handoff_messages,
    )
}

fn workflow_turn_delivery_token(workflow_node_run_id: &str) -> String {
    format!("workflow-ack:{workflow_node_run_id}")
}

#[cfg(test)]
mod tests;
