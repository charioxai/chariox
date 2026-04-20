use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::{SessionHistoryEntry, SessionHistoryEntryKind};
use crate::provider::{ControlOperation, LaunchProviderRequest, RuntimeProviderRun};
use crate::session::{
    PromptQueueItem, PromptSubmissionOutcome, RuntimeSession, WorkflowArtifactRef,
    WorkflowCompletionSnapshot, WorkflowCompletionUpdate, WorkflowConsole, WorkflowConsoleEntry,
    WorkflowDefinition, WorkflowDispatch, WorkflowFailureEvent, WorkflowFailureKind,
    WorkflowFailurePolicy, WorkflowFailurePolicyMode, WorkflowMessage, WorkflowNodeRunStatus,
    WorkflowOutputPayload, WorkflowRun, WorkflowRunStatus,
};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};
use arroba_relay::protocol::ClientTarget;

const WORKFLOW_PROMPT_SOURCE_PREFIX: &str = "workflow-run:";
const WORKFLOW_COMPLETION_SUMMARY_LIMIT: usize = 160;
const WORKFLOW_MAX_TURNS_CONFIG_KEY: &str = "workflow.max_turns";

#[derive(Debug, Deserialize)]
struct WorkflowStructuredOutputEnvelope {
    summary: Option<String>,
    output: Option<WorkflowStructuredOutputValue>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum WorkflowStructuredOutputValue {
    Text(String),
    Object { message: Value },
}

impl WorkflowStructuredOutputValue {
    fn into_output_message(self) -> Option<String> {
        match self {
            WorkflowStructuredOutputValue::Text(message) => {
                let trimmed = message.trim().to_string();
                (!trimmed.is_empty()).then_some(trimmed)
            }
            WorkflowStructuredOutputValue::Object { message } => match message {
                Value::String(message) => {
                    let trimmed = message.trim().to_string();
                    (!trimmed.is_empty()).then_some(trimmed)
                }
                other => Some(other.to_string()),
            },
        }
    }
}

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
        if let Err(error) = retry_prepared_workflow_node_prompt_without_provider_dispatch(
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

fn retry_prepared_workflow_node_prompt_without_provider_dispatch(
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
    match outcome {
        PromptSubmissionOutcome::Started { prompt } => {
            app.sessions_mut().mark_workflow_turn_dispatched(
                session_id,
                workflow_run_id,
                workflow_node_run_id,
            )?;
            on_workflow_prompt_started(app, session_id, &prompt)?;
            crate::transport::flow_control::note_prompt_started(app, &provider_run_id);
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

fn submit_claimed_workflow_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    prompt: &str,
) -> Result<PromptSubmissionOutcome, DaemonError> {
    let outcome = app.prompt_owner_submit_workflow_prompt(
        session_id,
        &workflow_prompt_source_attachment_id(workflow_run_id),
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
        prompt.to_string(),
    )?;
    Ok(outcome)
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

fn dispatch_workflow_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    target_agent_id: &str,
    prompt: &PromptQueueItem,
) -> Result<(), DaemonError> {
    let target_agent = app.agents().get_agent(target_agent_id)?;
    if let Some(remote_execution) = target_agent.remote_execution().cloned() {
        let workflow_context = crate::app::RemoteWorkflowTurnContextResolver::new(app)
            .remote_workflow_turn_context_for_prompt(session_id, target_agent_id, prompt)?;
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
                git_context: Some(crate::transport::relay_peer::RemoteGitTurnContext {
                    home_session_id: session_id.to_string(),
                    home_agent_id: target_agent_id.to_string(),
                    home_prompt_id: prompt.id().to_string(),
                    home_turn_id: prompt.id().to_string(),
                    prompt_summary: crate::prompt_transcript::render_prompt_transcript(
                        prompt.prompt(),
                        prompt.attachments(),
                    ),
                }),
                required_mcps: Vec::new(),
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
        crate::app::ProviderPromptDispatcher::new(app).dispatch_prompt_to_provider(
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
            crate::app::workflow_runtime::ensure_workflow_provider_run_from_runtime(
                app,
                session_id,
                target_agent_id,
            )?;
        match dispatch(app, &provider_run_id) {
            Ok(()) => {
                crate::transport::flow_control::note_prompt_started(app, &provider_run_id);
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
    match app.ensure_prompt_provider_run_for_agent(session_id, agent_id) {
        Ok(provider_run_id) => {
            let ended = app
                .providers()
                .get_run(&provider_run_id)
                .ok()
                .is_some_and(|run| run.state() == crate::provider::ProviderRunState::Ended);
            if ended {
                let agent = app.agents().get_agent(agent_id)?;
                let adapter_key = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let provider = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let mut request = LaunchProviderRequest::new(
                    session_id,
                    adapter_key,
                    provider,
                    "default",
                    agent.model().unwrap_or("default"),
                )
                .with_agent_id(agent.id().to_string())
                .with_variant(agent.effort().map(str::to_string));
                if crate::provider::provider_requires_managed_io_by_default(provider, app.config())
                {
                    request = request.with_managed_io_required();
                }
                if let Some(worktree_id) = agent.worktree_id() {
                    request = request.with_working_directory(PathBuf::from(worktree_id));
                }
                let provider_run = app.launch_provider_detached(request)?;
                app.sessions_mut()
                    .set_active_provider_run(session_id, Some(provider_run.id().to_string()))?;
                return Ok(provider_run.id().to_string());
            }
            app.sessions_mut()
                .set_active_provider_run(session_id, Some(provider_run_id.clone()))?;
            Ok(provider_run_id)
        }
        Err(DaemonError::NoActiveProviderRun { .. }) => {
            let agent = app.agents().get_agent(agent_id)?;
            let adapter_key = match agent.provider() {
                "default" => "opencode",
                value => value,
            };
            let provider = match agent.provider() {
                "default" => "opencode",
                value => value,
            };
            let mut request = LaunchProviderRequest::new(
                session_id,
                adapter_key,
                provider,
                "default",
                agent.model().unwrap_or("default"),
            )
            .with_agent_id(agent.id().to_string())
            .with_variant(agent.effort().map(str::to_string));
            if crate::provider::provider_requires_managed_io_by_default(provider, app.config()) {
                request = request.with_managed_io_required();
            }
            if let Some(worktree_id) = agent.worktree_id() {
                request = request.with_working_directory(PathBuf::from(worktree_id));
            }
            let provider_run = app.launch_provider_detached(request)?;
            app.sessions_mut()
                .set_active_provider_run(session_id, Some(provider_run.id().to_string()))?;
            Ok(provider_run.id().to_string())
        }
        Err(error) => Err(error),
    }
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
    if completion_snapshot.is_none() {
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

fn workflow_failure_policy() -> WorkflowFailurePolicy {
    WorkflowFailurePolicy::default()
}

fn provider_run_terminal_diagnostic(app: &DaemonApp, provider_run_id: &str) -> Option<String> {
    app.providers()
        .get_run(provider_run_id)
        .ok()
        .and_then(|run| run.terminal_diagnostic().map(str::to_string))
        .filter(|message| !message.trim().is_empty())
}

fn record_and_route_workflow_failure(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    failure: &WorkflowFailureEvent,
) {
    let _ = app.sessions_mut().record_workflow_failure_event(
        session_id,
        workflow_run_id,
        failure.clone(),
    );
    let policy = workflow_failure_policy();
    if policy.mode() != WorkflowFailurePolicyMode::Notify {
        return;
    }
    route_workflow_failure_mailboxes(app, session_id, workflow_run_id, failure, &policy);
}

fn route_workflow_failure_mailboxes(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    failure: &WorkflowFailureEvent,
    policy: &WorkflowFailurePolicy,
) {
    let workflow_run = match app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
    {
        Ok(run) => run,
        Err(_) => return,
    };
    let workflow = match app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
    {
        Ok(workflow) => workflow,
        Err(_) => return,
    };
    let Some(source_node_run) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == failure.source_node_run_id())
    else {
        return;
    };
    if policy.notify_source_node() {
        write_workflow_control_mailbox_entry(
            app,
            session_id,
            workflow_run_id,
            source_node_run.node_id(),
            failure,
        );
    }
    if !policy.notify_sink_nodes() {
        return;
    }
    for edge_id in failure.edge_ids() {
        let Some(edge) = workflow.edge(edge_id) else {
            continue;
        };
        write_workflow_control_mailbox_entry(
            app,
            session_id,
            workflow_run_id,
            edge.to_node_id(),
            failure,
        );
    }
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

fn workflow_node_control_contents(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
) -> Option<String> {
    let root = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-control",
    )?;
    let path = root.join(format!("node-{node_id}.md"));
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn workflow_runtime_artifact_root(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    category: &str,
) -> Option<std::path::PathBuf> {
    let base_directory =
        workflow_runtime_base_directory(app, session_id, workflow_run_id, workflow_node_run_id)?;
    Some(
        base_directory
            .join(".arroba")
            .join("workflow-runtime")
            .join(session_id)
            .join(workflow_run_id)
            .join(category),
    )
}

fn workflow_runtime_base_directory(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> Option<std::path::PathBuf> {
    let session = app.sessions().get_session(session_id).ok()?;
    let workflow_run = session.workflow_run(workflow_run_id)?;
    let node_run = workflow_run
        .node_runs()
        .iter()
        .find(|candidate| candidate.id() == workflow_node_run_id)?;
    let base_directory = app
        .providers()
        .get_latest_run_for_agent(session_id, node_run.agent_id())
        .and_then(|run| run.working_directory().cloned())
        .or_else(|| {
            let worktree = std::path::PathBuf::from(session.worktree_id());
            if worktree.is_absolute() {
                Some(worktree)
            } else {
                std::env::current_dir().ok().map(|cwd| cwd.join(worktree))
            }
        })?;
    Some(base_directory)
}

fn build_workflow_completion_snapshot(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    provider_run_id: Option<&str>,
) -> Option<WorkflowCompletionSnapshot> {
    let provider_run_id = provider_run_id
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let session = match app.sessions().get_session(session_id) {
        Ok(session) => session,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "failed to load session while building workflow completion snapshot",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_run_id": workflow_run_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
            return None;
        }
    };
    let Some(workflow_run) = session.workflow_run(workflow_run_id) else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let Some(_node_run) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
    else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow node run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let history = match crate::app::KernelSessionReadService::new(app).session_history(session_id) {
        Ok(history) => history,
        Err(error) => {
            crate::logging::warn_with_fields(
                "daemon.workflow",
                "failed to load session history for workflow completion snapshot",
                serde_json::json!({
                    "session_id": session_id,
                    "workflow_run_id": workflow_run_id,
                    "workflow_node_run_id": workflow_node_run_id,
                    "provider_run_id": provider_run_id,
                    "error": error.to_string(),
                }),
            );
            return None;
        }
    };
    build_workflow_completion_snapshot_from_history(
        &session,
        history,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        &provider_run_id,
    )
}

pub(crate) fn build_workflow_completion_snapshot_from_history(
    session: &RuntimeSession,
    history: Vec<SessionHistoryEntry>,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    provider_run_id: &str,
) -> Option<WorkflowCompletionSnapshot> {
    let Some(workflow_run) = session.workflow_run(workflow_run_id) else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let Some(node_run) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
    else {
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "workflow node run disappeared before completion snapshot could be built",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    };
    let started_at_ms = node_run
        .started_at_ms()
        .unwrap_or_else(|| node_run.created_at_ms());
    let output_started_at_ms = node_run
        .turn_envelope()
        .and_then(|envelope| {
            envelope
                .runtime_tool_calls()
                .iter()
                .rev()
                .find(|call| {
                    call.ok()
                        && call.tool_name()
                            == crate::transport::runtime_tools::ACK_WORKFLOW_TURN_TOOL
                })
                .map(|call| call.timestamp_ms())
        })
        .unwrap_or(started_at_ms);
    let provider_output = history
        .into_iter()
        .filter(|entry| {
            entry.provider_run_id.as_deref() == Some(provider_run_id)
                && entry.timestamp_ms >= output_started_at_ms
                && entry.kind == SessionHistoryEntryKind::ProviderOutput
        })
        .map(|entry| entry.text)
        .collect::<Vec<_>>()
        .join("");
    let structured_output = parse_workflow_structured_output(&provider_output);
    if structured_output.is_none() {
        if let Some(snapshot) =
            workflow_completion_snapshot_from_validated_tool_output(node_run, &provider_output)
        {
            return Some(snapshot);
        }
        crate::logging::warn_with_fields(
            "daemon.workflow",
            "ignoring workflow turn completion without structured output block",
            serde_json::json!({
                "session_id": session_id,
                "workflow_run_id": workflow_run_id,
                "workflow_node_run_id": workflow_node_run_id,
                "provider_run_id": provider_run_id,
            }),
        );
        return None;
    }
    let summary = structured_output
        .as_ref()
        .and_then(|value| value.summary.as_deref())
        .map(workflow_completion_summary)
        .unwrap_or_else(|| workflow_completion_summary(&provider_output));
    let artifacts = collect_workflow_artifact_refs(session_id, workflow_run_id, started_at_ms);
    let output_message = structured_output
        .as_ref()
        .and_then(|value| value.output.clone())
        .and_then(WorkflowStructuredOutputValue::into_output_message);
    let output = match (output_message, artifacts) {
        (Some(message), artifacts) => Some(WorkflowOutputPayload::new(message, artifacts)),
        (None, artifacts) if !artifacts.is_empty() => {
            Some(WorkflowOutputPayload::new("artifacts attached", artifacts))
        }
        _ => None,
    };
    if summary == "completed" && output.is_none() {
        return None;
    }

    Some(WorkflowCompletionSnapshot::new(summary, output))
}

fn workflow_completion_snapshot_from_validated_tool_output(
    node_run: &crate::session::WorkflowNodeRun,
    provider_output: &str,
) -> Option<WorkflowCompletionSnapshot> {
    let call = node_run
        .turn_envelope()?
        .runtime_tool_calls()
        .iter()
        .rev()
        .find(|call| {
            call.ok()
                && call.tool_name()
                    == crate::transport::runtime_tools::VALIDATE_WORKFLOW_OUTPUT_TOOL
                && call
                    .result_json()
                    .and_then(|result| serde_json::from_str::<serde_json::Value>(result).ok())
                    .and_then(|value| value.get("valid").and_then(|valid| valid.as_bool()))
                    == Some(true)
        })?;
    let args = serde_json::from_str::<crate::transport::runtime_tools::ValidateWorkflowOutputArgs>(
        call.arguments_json(),
    )
    .ok()?;
    let summary = workflow_completion_summary(provider_output);
    Some(WorkflowCompletionSnapshot::new(
        summary,
        Some(WorkflowOutputPayload::new(args.output_json, Vec::new())),
    ))
}

fn write_workflow_control_mailbox_entry(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
    failure: &WorkflowFailureEvent,
) {
    let Some(root) = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        failure.source_node_run_id(),
        "workflow-control",
    ) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(&root) {
        tracing::debug!(
            ?error,
            "Failed to create workflow control directory at {:?}",
            root
        );
        return;
    }
    let path = root.join(format!("node-{node_id}.md"));
    let existing = std::fs::read_to_string(&path).ok();
    let header = "# Workflow Control Mailbox\n\n";
    let existing_body = existing
        .as_deref()
        .unwrap_or("")
        .strip_prefix(header)
        .unwrap_or("")
        .trim();
    let edge_label = if failure.edge_ids().is_empty() {
        "node-local".to_string()
    } else {
        failure
            .edge_ids()
            .iter()
            .map(|edge_id| format!("edge {edge_id}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let entry = format!(
        "- [{} @ {}] {}: {}",
        match failure.kind() {
            WorkflowFailureKind::MissingAck => "missing_ack",
            WorkflowFailureKind::MissingStructuredOutput => "missing_structured_output",
            WorkflowFailureKind::OutputValidationFailed => "output_validation_failed",
            WorkflowFailureKind::WorkflowRunOutputValidationFailed => {
                "workflow_run_output_validation_failed"
            }
            WorkflowFailureKind::NodeTurnBudgetExhausted => "node_turn_budget_exhausted",
            WorkflowFailureKind::RunStopped => "run_stopped",
            WorkflowFailureKind::ProviderFailure => "provider_failure",
            WorkflowFailureKind::TransportFailure => "transport_failure",
            WorkflowFailureKind::TurnStalled => "turn_stalled",
        },
        failure.timestamp_ms(),
        edge_label,
        failure.message()
    );
    let body = if existing_body.is_empty() {
        entry
    } else if existing_body.lines().any(|line| line.trim() == entry) {
        existing_body.to_string()
    } else {
        format!("{existing_body}\n{entry}")
    };
    let content = format!("{header}Notifications for node `{node_id}`:\n{body}\n");
    if let Err(error) = std::fs::write(&path, content) {
        tracing::debug!(
            ?error,
            "Failed to write workflow control mailbox at {:?}",
            path
        );
    }
}

fn clear_workflow_control_mailbox(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    workflow_run: &WorkflowRun,
) {
    let Some(node_id) = workflow_run
        .node_runs()
        .iter()
        .find(|node_run| node_run.id() == workflow_node_run_id)
        .map(|node_run| node_run.node_id())
    else {
        return;
    };
    let Some(root) = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-control",
    ) else {
        return;
    };
    let path = root.join(format!("node-{node_id}.md"));
    let _ = std::fs::remove_file(path);
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

    use super::{parse_workflow_structured_output, prepare_workflow_turn_prompt};

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

    #[test]
    fn workflow_structured_output_accepts_json_message_values() {
        let parsed = parse_workflow_structured_output(
            r#"
```json
{"summary":"fixed","output":{"message":{"ok":true,"source":"mailbox-fixed"}}}
```
"#,
        )
        .expect("structured output should parse");

        let output = parsed
            .output
            .expect("structured output should contain output")
            .into_output_message()
            .expect("message should serialize");
        assert_eq!(output, r#"{"ok":true,"source":"mailbox-fixed"}"#);
    }
}

fn collect_workflow_artifact_refs(
    session_id: &str,
    workflow_run_id: &str,
    started_at_ms: u64,
) -> Vec<WorkflowArtifactRef> {
    let attachment_id = workflow_prompt_source_attachment_id(workflow_run_id);
    let mut artifacts = Vec::new();
    for root in DaemonApp::attachment_artifact_roots(session_id, &attachment_id) {
        let kind = root
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|value| value.to_str())
            .unwrap_or("artifact")
            .trim_end_matches('s')
            .to_string();
        collect_workflow_artifacts_from_dir(&root, &kind, started_at_ms, &mut artifacts);
    }
    artifacts.sort_by(|left, right| left.id().cmp(right.id()));
    artifacts
}

fn workflow_completion_summary(source: &str) -> String {
    if source.trim().is_empty() {
        return "completed".to_string();
    }
    let normalized = source
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        return "completed".to_string();
    }
    if normalized.chars().count() <= WORKFLOW_COMPLETION_SUMMARY_LIMIT {
        return normalized;
    }

    let truncated = normalized
        .chars()
        .take(WORKFLOW_COMPLETION_SUMMARY_LIMIT)
        .collect::<String>();
    format!("{truncated}...")
}

fn parse_workflow_structured_output(text: &str) -> Option<WorkflowStructuredOutputEnvelope> {
    let mut cursor = 0usize;
    let mut parsed = None;
    while let Some(start) = text[cursor..].find("```json") {
        let block_start = cursor + start + "```json".len();
        let remaining = &text[block_start..];
        let Some(end) = remaining.find("```") else {
            break;
        };
        let candidate = remaining[..end].trim();
        if let Ok(value) = serde_json::from_str::<WorkflowStructuredOutputEnvelope>(candidate) {
            parsed = Some(value);
        }
        cursor = block_start + end + "```".len();
    }
    parsed
}

fn collect_workflow_artifacts_from_dir(
    root: &std::path::Path,
    kind: &str,
    started_at_ms: u64,
    artifacts: &mut Vec<WorkflowArtifactRef>,
) {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_workflow_artifacts_from_dir(&path, kind, started_at_ms, artifacts);
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let modified_at_ms = modified
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        if modified_at_ms < started_at_ms {
            continue;
        }
        let display_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("artifact")
            .to_string();
        let path_string = path.to_string_lossy().into_owned();
        artifacts.push(WorkflowArtifactRef::new(
            format!("{kind}:{display_name}"),
            kind.to_string(),
            path_string,
            display_name,
        ));
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
