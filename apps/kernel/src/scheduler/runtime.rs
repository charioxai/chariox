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
mod prompt_dispatch;

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
            "",
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
    let handoff_payloads_json = prompt
        .split("Workflow handoff payloads (JSON array):\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "[]")
        .map(str::to_string);
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
                        crate::transport::flow_control::clear_active_turn(app, provider_run_id);
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

fn workflow_node_run_has_valid_pending_final_output(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> bool {
    app.sessions()
        .get_session(session_id)
        .ok()
        .and_then(|session| {
            session
                .workflow_run(workflow_run_id)
                .and_then(|workflow_run| {
                    workflow_run
                        .node_runs()
                        .iter()
                        .find(|node_run| node_run.id() == workflow_node_run_id)
                })
                .map(|node_run| node_run.has_valid_pending_final_output())
        })
        .unwrap_or(false)
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
    prompt_dispatch::ensure_workflow_provider_run_for_agent(app, session_id, agent_id)
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
    let completion_snapshot = build_workflow_completion_snapshot(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        provider_run_id,
    );
    let has_valid_pending_final_output = workflow_node_run_has_valid_pending_final_output(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    );
    if completion_snapshot.is_none() && !has_valid_pending_final_output {
        let message = "provider completed workflow turn without a validated workflow output";
        let provider_diagnostic =
            provider_run_id.and_then(|run_id| provider_run_terminal_diagnostic(app, run_id));
        let (failure_kind, failure_message, notice_message) = if let Some(diagnostic) =
            provider_diagnostic
        {
            (
                WorkflowFailureKind::ProviderFailure,
                diagnostic.clone(),
                format!(
                    "Workflow run `{workflow_run_id}` failed after provider turn failure: {diagnostic}"
                ),
            )
        } else {
            (
                WorkflowFailureKind::MissingStructuredOutput,
                message.to_string(),
                format!("Workflow run `{workflow_run_id}` failed: {message}."),
            )
        };
        app.sessions_mut().fail_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        record_and_route_workflow_failure(
            app,
            session_id,
            workflow_run_id,
            &WorkflowFailureEvent::new(
                failure_kind,
                workflow_node_run_id,
                Vec::new(),
                failure_message,
            ),
        );
        app.record_notice(
            session_id,
            provider_run_id,
            app.attachments().list_session_attachment_ids(session_id),
            notice_message,
        );
        maybe_start_next_queued_workflow_launch(app, session_id);
        let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id);
        return Ok(());
    }
    let max_turns = workflow_max_turns(app, session_id);
    let completion_result = {
        app.sessions_mut().complete_workflow_node_run(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
            completion_snapshot.clone(),
            max_turns,
        )
    };
    let WorkflowCompletionUpdate {
        workflow_run,
        dispatches,
        validation_warnings,
    } = match completion_result {
        Ok(update) => update,
        Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
            edge_id, message, ..
        }) => {
            record_and_route_workflow_failure(
                app,
                session_id,
                workflow_run_id,
                &WorkflowFailureEvent::new(
                    WorkflowFailureKind::OutputValidationFailed,
                    workflow_node_run_id,
                    vec![edge_id.clone()],
                    message.clone(),
                ),
            );
            app.sessions_mut().stop_workflow_node_run(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                ),
            );
            maybe_start_next_queued_workflow_launch(app, session_id);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if !validation_warnings.is_empty() {
        for warning in &validation_warnings {
            let failure = WorkflowFailureEvent::new(
                crate::session::classify_workflow_failure_kind(
                    &completion_snapshot,
                    &warning.message,
                ),
                workflow_node_run_id,
                vec![warning.edge_id.clone()],
                warning.message.clone(),
            );
            record_and_route_workflow_failure(app, session_id, workflow_run_id, &failure);
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
    if workflow_run.status() == WorkflowRunStatus::Stopped
        && workflow_run.final_output().is_none()
        && workflow_run
            .failure_events()
            .iter()
            .all(|event| event.kind() != WorkflowFailureKind::NodeTurnBudgetExhausted)
    {
        record_and_route_workflow_failure(
            app,
            session_id,
            workflow_run_id,
            &WorkflowFailureEvent::new(
                WorkflowFailureKind::NodeTurnBudgetExhausted,
                workflow_node_run_id,
                Vec::new(),
                "workflow run stopped after a node exhausted its turn budget",
            ),
        );
    }
    if workflow_run.final_output_valid() == Some(false) {
        record_and_route_workflow_failure(
            app,
            session_id,
            workflow_run_id,
            &WorkflowFailureEvent::new(
                WorkflowFailureKind::WorkflowRunOutputValidationFailed,
                workflow_node_run_id,
                Vec::new(),
                workflow_run
                    .final_output_warning()
                    .unwrap_or("workflow run output validation failed"),
            ),
        );
    }
    if validation_warnings.is_empty() {
        let updated = app.sessions_mut().mark_workflow_turn_validated_completed(
            session_id,
            workflow_run_id,
            workflow_node_run_id,
        )?;
        if updated
            .node_runs()
            .iter()
            .find(|node_run| node_run.id() == workflow_node_run_id)
            .and_then(|node_run| node_run.turn_envelope())
            .is_some_and(|envelope| {
                envelope.state() == crate::session::WorkflowTurnRuntimeState::ValidatedCompleted
            })
        {
            clear_workflow_control_mailbox(
                app,
                session_id,
                workflow_run_id,
                workflow_node_run_id,
                &updated,
            );
        }
    }
    let claim_provider_run_id = provider_run_id.map(str::to_string).or_else(|| {
        app.providers()
            .get_run_for_agent(session_id, prompt.target_agent_id())
            .map(|run| run.id().to_string())
    });
    let released_claim = claim_provider_run_id
        .as_deref()
        .map(|provider_run_id| app.release_prompt_workspace_claim(provider_run_id))
        .unwrap_or(false);
    let released_workflow_claim = app.release_workflow_node_workspace_claim(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    );
    schedule_workflow_dispatches(app, session_id, workflow_run.id(), &dispatches);
    if released_claim || released_workflow_claim {
        let _ = retry_blocked_workflow_claims(app);
    }
    let state_suffix = match workflow_run.status() {
        WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
        WorkflowRunStatus::Completing => "is completing",
        WorkflowRunStatus::Completed => "completed",
        WorkflowRunStatus::Stopped => "stopped",
        _ => "updated",
    };
    app.record_notice(
        session_id,
        None,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{}` {state_suffix}.", workflow_run.id()),
    );
    if matches!(
        workflow_run.status(),
        WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Stopped
    ) {
        maybe_start_next_queued_workflow_launch(app, session_id);
    }
    Ok(())
}

pub fn on_workflow_provider_failure(
    app: &mut DaemonApp,
    session_id: &str,
    prompt: &PromptQueueItem,
    provider_run_id: Option<&str>,
    message: &str,
) -> Result<(), DaemonError> {
    let (Some(workflow_run_id), Some(workflow_node_run_id)) =
        (prompt.workflow_run_id(), prompt.workflow_node_run_id())
    else {
        return Ok(());
    };
    record_and_route_workflow_failure(
        app,
        session_id,
        workflow_run_id,
        &WorkflowFailureEvent::new(
            WorkflowFailureKind::ProviderFailure,
            workflow_node_run_id,
            Vec::new(),
            message,
        ),
    );
    app.sessions_mut()
        .fail_workflow_node_run(session_id, workflow_run_id, workflow_node_run_id)?;
    app.record_notice(
        session_id,
        provider_run_id,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{workflow_run_id}` failed after provider turn failure: {message}"),
    );
    maybe_start_next_queued_workflow_launch(app, session_id);
    let _ = crate::app::KernelSessionReadService::new(app).session_snapshot(session_id);
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
    record_and_route_workflow_failure(
        app,
        session_id,
        workflow_run_id,
        &WorkflowFailureEvent::new(
            WorkflowFailureKind::RunStopped,
            workflow_node_run_id,
            Vec::new(),
            "workflow node run was stopped before validated completion",
        ),
    );
    app.record_notice(
        session_id,
        None,
        app.attachments().list_session_attachment_ids(session_id),
        format!("Workflow run `{}` was stopped.", workflow_run.id()),
    );
    maybe_start_next_queued_workflow_launch(app, session_id);
    Ok(())
}

fn maybe_start_next_queued_workflow_launch(app: &mut DaemonApp, session_id: &str) {
    match app.drain_session_workflow_launch_queue(session_id) {
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
        Ok(Some(crate::app::workflow_runtime::WorkflowLaunchOutcome::Queued { .. })) => {}
        Ok(None) => {}
        Err(error) => {
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!("Failed to start queued workflow launch: {error}"),
            );
        }
    }
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
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::agent::CreateAgentRequest;
    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::provider::LaunchProviderRequest;
    use crate::session::{CreateSessionRequest, RuntimeSession, WorkflowMessage, WorkflowRun};
    use crate::{DaemonApp, DaemonConfig};

    use super::prepare_workflow_turn_prompt;

    fn create_scheduler_session_and_agent(
        app: &mut DaemonApp,
        client_id: &str,
    ) -> (RuntimeSession, String) {
        let (session, _default_agent) = crate::app::KernelSessionService::new(&mut *app)
            .create_session(CreateSessionRequest::new(
                "workspace-scheduler",
                "worktree-scheduler",
            ))
            .expect("session should exist");
        crate::app::KernelSessionService::new(app)
            .attach(AttachRequest::new(
                session.id(),
                client_id,
                ClientCapabilityLevel::InteractiveStructured,
            ))
            .expect("attachment should attach");
        let agent_id = crate::app::KernelSessionService::new(&mut *app)
            .spawn_agent(
                CreateAgentRequest::new(session.id(), "dev-stub")
                    .with_alias("agent-scheduler")
                    .with_model("test-model")
                    .with_worktree("worktree-scheduler"),
            )
            .expect("agent should spawn")
            .id()
            .to_string();
        (session, agent_id)
    }

    fn create_workflow_node(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_alias: &str,
        agent_id: &str,
    ) -> (String, String) {
        let workflow_id = app
            .sessions_mut()
            .create_workflow(session_id, Some(workflow_alias.to_string()))
            .expect("workflow should exist")
            .id()
            .to_string();
        let node_id = app
            .sessions_mut()
            .add_workflow_node(session_id, &workflow_id, agent_id)
            .expect("node should be added")
            .id()
            .to_string();
        (workflow_id, node_id)
    }

    fn invoke_workflow_node(
        app: &mut DaemonApp,
        session_id: &str,
        workflow_id: &str,
        node_id: &str,
    ) -> WorkflowRun {
        app.sessions_mut()
            .set_workflow_flush_agent_context_before_run(session_id, &workflow_id, false)
            .expect("workflow flush context should update");
        app.sessions_mut()
            .create_workflow_endpoint(
                session_id,
                &workflow_id,
                &node_id,
                Some("entry".to_string()),
            )
            .expect("endpoint should exist");
        let (workflow_run, _, _) = app
            .invoke_workflow_endpoint_and_schedule(
                session_id,
                &workflow_id,
                "entry",
                Some("start".to_string()),
            )
            .expect("workflow should invoke");
        workflow_run
    }

    #[test]
    fn workflow_instruction_reference_is_written_under_agent_workdir() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent_id) = create_scheduler_session_and_agent(&mut app, "client-scheduler");

        let workdir = std::env::temp_dir().join(format!(
            "arroba-workflow-runtime-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&workdir);
        fs::create_dir_all(&workdir).expect("workdir should exist");
        app.launch_provider(
            LaunchProviderRequest::new(
                session.id(),
                "dev-stub",
                "dev-stub",
                "default",
                "test-model",
            )
            .with_agent_id(agent_id.clone())
            .with_working_directory(workdir.clone()),
        )
        .expect("provider run should launch");

        let workflow_id = app
            .sessions_mut()
            .create_workflow(session.id(), Some("wf-scheduler".to_string()))
            .expect("workflow should exist")
            .id()
            .to_string();
        let node_id = app
            .sessions_mut()
            .add_workflow_node(session.id(), &workflow_id, &agent_id)
            .expect("node should be added")
            .id()
            .to_string();
        app.sessions_mut()
            .update_workflow_node_instructions(
                session.id(),
                &workflow_id,
                &node_id,
                Some("Read me from a workspace-local hidden file.".to_string()),
            )
            .expect("instructions should update");
        app.sessions_mut()
            .set_workflow_flush_agent_context_before_run(session.id(), &workflow_id, false)
            .expect("workflow flush context should update");
        app.sessions_mut()
            .create_workflow_endpoint(
                session.id(),
                &workflow_id,
                &node_id,
                Some("entry".to_string()),
            )
            .expect("endpoint should exist");
        let (workflow_run, _, _) = app
            .invoke_workflow_endpoint_and_schedule(
                session.id(),
                &workflow_id,
                "entry",
                Some("start".to_string()),
            )
            .expect("workflow should invoke");
        let node_run_id = workflow_run
            .node_runs()
            .first()
            .expect("node run should exist")
            .id()
            .to_string();

        let prompt = prepare_workflow_turn_prompt(
            &app,
            session.id(),
            workflow_run.id(),
            &node_run_id,
            &node_id,
            "start",
            Option::<&[WorkflowMessage]>::None,
        )
        .expect("prompt should build");

        let prefix = workdir
            .join(".arroba")
            .join("workflow-runtime")
            .join(session.id())
            .join(workflow_run.id())
            .join("workflow-instructions");
        let prefix_string = prefix.to_string_lossy().to_string();
        assert!(
            prompt.contains(&prefix_string),
            "prompt should reference a file under agent workdir: {prompt}"
        );
        let expected_file = prefix.join(format!("node-{node_id}.md"));
        assert!(expected_file.exists(), "instruction file should be written");
        let contents = fs::read_to_string(&expected_file).expect("instruction file should read");
        assert!(contents.contains("Read me from a workspace-local hidden file."));
        let expected_prompt_template = workdir
            .join(".arroba")
            .join("system-prompts")
            .join("workflow-turn.md");
        assert!(
            expected_prompt_template.exists(),
            "workflow system prompt template should be materialized"
        );
        let prompt_template_contents =
            fs::read_to_string(&expected_prompt_template).expect("template should read");
        assert!(prompt_template_contents.contains("ack_workflow_turn"));
        assert!(prompt
            .contains("If you do not remember them exactly, read that file before continuing."));
        let _ = fs::remove_dir_all(PathBuf::from(workdir));
    }

    #[test]
    fn terminating_nodes_receive_completion_and_last_turn_prompt_blocks() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent_id) =
            create_scheduler_session_and_agent(&mut app, "client-scheduler-terminating");

        let (workflow_id, node_id) = create_workflow_node(
            &mut app,
            session.id(),
            "wf-scheduler-terminating",
            &agent_id,
        );
        app.sessions_mut()
            .set_workflow_node_can_complete_run(session.id(), &workflow_id, &node_id, true)
            .expect("node completion setting should update");
        app.sessions_mut()
            .set_workflow_node_max_turns(session.id(), &workflow_id, &node_id, Some(1))
            .expect("node max turns should update");
        let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
        let node_run_id = workflow_run
            .node_runs()
            .first()
            .expect("node run should exist")
            .id()
            .to_string();
        let prompt = prepare_workflow_turn_prompt(
            &app,
            session.id(),
            workflow_run.id(),
            &node_run_id,
            &node_id,
            "start",
            Option::<&[WorkflowMessage]>::None,
        )
        .expect("prompt should build");

        assert!(prompt.contains("This node is authorized to complete the workflow run."));
        assert!(prompt.contains("This is turn 1 for this node in the current workflow run."));
        assert!(prompt
            .contains("This is the last allowed turn for this node in the current workflow run."));
        assert!(prompt.contains("validate_and_submit_workflow_run_output"));
    }

    #[test]
    fn non_last_turn_nodes_still_receive_turn_index_prompt_block() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, agent_id) =
            create_scheduler_session_and_agent(&mut app, "client-scheduler-turn-index");

        let (workflow_id, node_id) =
            create_workflow_node(&mut app, session.id(), "wf-scheduler-turn-index", &agent_id);
        app.sessions_mut()
            .set_workflow_node_max_turns(session.id(), &workflow_id, &node_id, Some(3))
            .expect("node max turns should update");
        let workflow_run = invoke_workflow_node(&mut app, session.id(), &workflow_id, &node_id);
        let node_run_id = workflow_run
            .node_runs()
            .first()
            .expect("node run should exist")
            .id()
            .to_string();
        let prompt = prepare_workflow_turn_prompt(
            &app,
            session.id(),
            workflow_run.id(),
            &node_run_id,
            &node_id,
            "start",
            Option::<&[WorkflowMessage]>::None,
        )
        .expect("prompt should build");

        assert!(prompt.contains("This is turn 1 for this node in the current workflow run."));
        assert!(prompt.contains("- node max turns: 3"));
        assert!(!prompt
            .contains("This is the last allowed turn for this node in the current workflow run."));
    }
}

pub fn validate_workflow_agents(
    app: &DaemonApp,
    session_id: &str,
    workflow: &WorkflowDefinition,
) -> Result<(), DaemonError> {
    let agents = app
        .agents()
        .get_session_agents(session_id)
        .into_iter()
        .collect::<Vec<_>>();
    let agent_ids = agents
        .iter()
        .map(|agent| agent.id().to_string())
        .collect::<BTreeSet<_>>();
    for node in workflow.nodes() {
        if !agent_ids.contains(node.agent_id()) {
            return Err(DaemonError::WorkflowNodeAgentMissing {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node.id().to_string(),
                agent_id: node.agent_id().to_string(),
            });
        }
        let Some(agent) = agents.iter().find(|agent| agent.id() == node.agent_id()) else {
            continue;
        };
        let capabilities =
            workflow_node_control_capabilities(app, session_id, node.agent_id(), agent.provider());
        if !capabilities.supports_control_operation(ControlOperation::AckWorkflowTurn) {
            return Err(DaemonError::WorkflowNodeControlUnsupported {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node.id().to_string(),
                agent_id: node.agent_id().to_string(),
                operation: "ack_workflow_turn",
            });
        }
        let requires_validation = workflow
            .edges()
            .iter()
            .any(|edge| edge.from_node_id() == node.id() && edge.output_schema_ref().is_some());
        if requires_validation
            && !capabilities.supports_control_operation(ControlOperation::ValidateWorkflowOutput)
        {
            return Err(DaemonError::WorkflowNodeControlUnsupported {
                session_id: session_id.to_string(),
                workflow_id: workflow.id().to_string(),
                node_id: node.id().to_string(),
                agent_id: node.agent_id().to_string(),
                operation: "validate_workflow_output",
            });
        }
    }
    Ok(())
}

fn workflow_node_control_capabilities(
    app: &DaemonApp,
    session_id: &str,
    agent_id: &str,
    provider: &str,
) -> RuntimeProviderRun {
    if let Some(run) = app.providers().get_run_for_agent(session_id, agent_id) {
        return run;
    }

    RuntimeProviderRun::from_control_capability_inference(
        format!("inferred-{session_id}-{agent_id}"),
        session_id.to_string(),
        Some(agent_id.to_string()),
        provider.to_string(),
    )
}
