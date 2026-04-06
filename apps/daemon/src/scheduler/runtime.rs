use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde_json::Value;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::history::SessionHistoryEntryKind;
use crate::provider::{ControlOperation, LaunchProviderRequest, RuntimeProviderRun};
use crate::session::{
    PromptQueueItem, PromptSubmissionOutcome, WorkflowArtifactRef, WorkflowCompletionSnapshot,
    WorkflowCompletionUpdate, WorkflowConsole, WorkflowConsoleEntry, WorkflowDefinition,
    WorkflowDispatch, WorkflowFailureEvent, WorkflowFailureKind, WorkflowFailurePolicy,
    WorkflowFailurePolicyMode, WorkflowMessage, WorkflowNodeRunStatus, WorkflowOutputPayload,
    WorkflowOutputValidationPolicy, WorkflowRun, WorkflowRunStatus,
};
use crate::transport::TransportService;

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
    let (_session, outcome) = app.sessions_mut().submit_workflow_prompt(
        session_id,
        &workflow_prompt_source_attachment_id(workflow_run_id),
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
                if let Ok(Some(cancelled)) =
                    TransportService::cancel_active_prompt_after_dispatch_failure(app, session_id)
                {
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

pub fn resume_workflow_run(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_ref: &str,
) -> Result<WorkflowRun, DaemonError> {
    let workflow_run = app.sessions_mut().resume_workflow_run(session_id, workflow_run_ref)?;
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
                node_run.agent_id().to_string(),
                node_run
                    .turn_envelope()
                    .and_then(|envelope| envelope.rendered_prompt())
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect::<Vec<_>>();
    for (workflow_node_run_id, agent_id, prompt) in resumable_node_runs {
        resume_existing_workflow_node_prompt(
            app,
            session_id,
            workflow_run.id(),
            &workflow_node_run_id,
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
    target_agent_id: &str,
    prompt: &str,
) -> Result<(), DaemonError> {
    let (_session, outcome) = app.sessions_mut().submit_workflow_prompt(
        session_id,
        &workflow_prompt_source_attachment_id(workflow_run_id),
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
        prompt.to_string(),
    )?;

    match outcome {
        PromptSubmissionOutcome::Started { prompt } => {
            TransportService::dispatch_workflow_prompt(app, session_id, target_agent_id, &prompt)?;
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
                    "Workflow run `{workflow_run_id}` queued resumed node run `{workflow_node_run_id}` behind the current active prompt."
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
    match app.ensure_active_provider_run_for_agent(session_id, agent_id) {
        Ok(provider_run_id) => Ok(provider_run_id),
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
            if let Some(worktree_id) = agent.worktree_id() {
                request = request.with_working_directory(PathBuf::from(worktree_id));
            }
            let provider_run = app.launch_provider(request)?;
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
    let max_turns = workflow_max_turns(app, session_id);
    let WorkflowCompletionUpdate {
        workflow_run,
        dispatches,
        validation_warnings,
    } = match app.sessions_mut().complete_workflow_node_run(
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        completion_snapshot.clone(),
        max_turns,
    ) {
        Ok(update) => update,
        Err(crate::error::DaemonError::WorkflowOutputValidationFailed {
            edge_id,
            message,
            ..
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
            app.sessions_mut()
                .stop_workflow_node_run(session_id, workflow_run_id, workflow_node_run_id)?;
            app.record_notice(
                session_id,
                None,
                app.attachments().list_session_attachment_ids(session_id),
                format!(
                    "Workflow run `{workflow_run_id}` stopped after validation failed on edge `{edge_id}`: {message}"
                ),
            );
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
    schedule_workflow_dispatches(app, session_id, workflow_run.id(), &dispatches);
    let state_suffix = match workflow_run.status() {
        WorkflowRunStatus::Waiting => "waiting for downstream handoffs",
        WorkflowRunStatus::Completed => "completed",
        WorkflowRunStatus::Stopped => "stopped after reaching the max turn limit",
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
    Ok(())
}

pub fn read_workflow_console(
    app: &DaemonApp,
    session_id: &str,
    workflow_id: &str,
) -> Result<WorkflowConsole, DaemonError> {
    app.sessions().read_workflow_console(session_id, workflow_id)
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
    app.sessions_mut().clear_workflow_console(session_id, workflow_id)
}

fn workflow_failure_policy() -> WorkflowFailurePolicy {
    WorkflowFailurePolicy::default()
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
    let workflow_run = match app.sessions().resolve_workflow_run_ref(session_id, workflow_run_id) {
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
    let instruction_ref = workflow_node_instruction_reference(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
    );
    let mailbox_content = workflow_node_control_contents(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
    );
    let handoff_payloads_json = serialize_handoff_payloads_json(handoff_messages);
    let delivery_token = workflow_turn_delivery_token(workflow_node_run_id);
    Ok(build_workflow_turn_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        node_id,
        endpoint_prompt,
        instruction_ref,
        mailbox_content,
        handoff_payloads_json,
        &delivery_token,
    ))
}

fn build_workflow_turn_prompt(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
    endpoint_prompt: &str,
    instruction_ref: Option<String>,
    mailbox_content: Option<String>,
    handoff_payloads_json: Option<String>,
    delivery_token: &str,
) -> String {
    let reference_line = instruction_ref
        .as_deref()
        .map(|path| format!("Node instruction reference (daemon-managed): {path}\n\n"))
        .unwrap_or_default();
    let control_line = mailbox_content
        .as_deref()
        .map(|content| {
            format!(
                "Control mailbox:\n{content}\nTreat the control mailbox as authoritative runtime feedback for this node. Fix every listed issue in this turn before you finalize the workflow output.\n\n"
            )
        })
        .unwrap_or_default();
    let workflow_prompt = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()
        .and_then(|run| run.invocation_prompt().map(str::to_string))
        .unwrap_or_default();
    let payload_block = if handoff_payloads_json
        .as_deref()
        .is_none_or(|payloads| payloads.trim().is_empty() || payloads.trim() == "[]")
    {
        String::new()
    } else {
        format!(
            "Workflow handoff payloads (JSON array):\n{}\n\n",
            handoff_payloads_json.as_deref().unwrap_or("[]")
        )
    };
    let edge_contract_block =
        workflow_outgoing_edge_contracts_block(app, session_id, workflow_run_id, node_id);
    let entry_line = if endpoint_prompt.trim().is_empty() {
        String::new()
    } else {
        format!("Endpoint prompt:\n{endpoint_prompt}\n\n")
    };
    let system_prompt = render_workflow_system_prompt(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        delivery_token,
        &payload_block,
        &edge_contract_block,
        &reference_line,
        &control_line,
    );
    format!(
        "{}Workflow-level prompt:\n{}\n\n{}\n",
        entry_line,
        workflow_prompt,
        system_prompt
    )
}

fn render_workflow_system_prompt(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    delivery_token: &str,
    payload_block: &str,
    edge_contract_block: &str,
    reference_line: &str,
    control_line: &str,
) -> String {
    let template = load_workflow_system_prompt_template(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
    );
    template
        .replace("{{DELIVERY_TOKEN}}", delivery_token)
        .replace("{{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}", payload_block)
        .replace("{{OUTGOING_EDGE_CONTRACTS_BLOCK}}", edge_contract_block)
        .replace("{{NODE_INSTRUCTION_REFERENCE_BLOCK}}", reference_line)
        .replace("{{CONTROL_MAILBOX_BLOCK}}", control_line)
}

fn load_workflow_system_prompt_template(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> String {
    let Some(path) =
        workflow_system_prompt_template_path(app, session_id, workflow_run_id, workflow_node_run_id)
    else {
        return default_workflow_system_prompt_template().to_string();
    };
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, default_workflow_system_prompt_template());
    }
    std::fs::read_to_string(&path)
        .unwrap_or_else(|_| default_workflow_system_prompt_template().to_string())
}

fn workflow_system_prompt_template_path(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
) -> Option<std::path::PathBuf> {
    let base_directory =
        workflow_runtime_base_directory(app, session_id, workflow_run_id, workflow_node_run_id)?;
    Some(
        base_directory
            .join(".arroba")
            .join("system-prompts")
            .join("workflow-turn.md"),
    )
}

fn workflow_outgoing_edge_contracts_block(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    node_id: &str,
) -> String {
    let Some(workflow_run) = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()
    else {
        return String::new();
    };
    let Some(workflow) = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
        .ok()
    else {
        return String::new();
    };

    let lines = workflow
        .edges()
        .iter()
        .filter(|edge| edge.from_node_id() == node_id)
        .map(|edge| {
            let mut line = format!("- edge {} -> {}", edge.id(), edge.to_node_id());
            if let Some(schema_ref) = edge.output_schema_ref() {
                line.push_str(&format!(", output_schema_ref: {schema_ref}"));
            }
            if let Some(validation_policy) = edge.validation_policy() {
                let validation_policy = match validation_policy {
                    WorkflowOutputValidationPolicy::Warn => "warn",
                    WorkflowOutputValidationPolicy::Halt => "halt",
                };
                line.push_str(&format!(", validation_policy: {validation_policy}"));
            }
            line
        })
        .collect::<Vec<String>>();

    if lines.is_empty() {
        return String::new();
    }

    format!(
        "Outgoing edge contracts:\n{}\nAll schema refs needed for this turn are listed above. Do not search the workspace for workflow metadata unless the workflow-level prompt explicitly asks you to.\n\n",
        lines.join("\n")
    )
}

fn workflow_node_instruction_reference(
    app: &DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    node_id: &str,
) -> Option<String> {
    let workflow_run = app
        .sessions()
        .resolve_workflow_run_ref(session_id, workflow_run_id)
        .ok()?;
    let workflow = app
        .sessions()
        .resolve_workflow_ref(session_id, workflow_run.workflow_id())
        .ok()?;
    let node = workflow.node(node_id);
    let root = workflow_runtime_artifact_root(
        app,
        session_id,
        workflow_run_id,
        workflow_node_run_id,
        "workflow-instructions",
    )?;
    let filename = format!("node-{node_id}.md");
    let path = root.join(filename);
    if !path.exists() || node.and_then(|node| node.instructions()).is_some() {
        if let Err(error) = std::fs::create_dir_all(&root) {
            tracing::debug!(
                ?error,
                "Failed to create workflow instruction directory at {:?}",
                root
            );
            return None;
        }
        let content = node
            .and_then(|node| node.instructions())
            .map(|value| value.to_string())
            .unwrap_or_else(|| {
                format!(
                    "# Workflow Node Instructions\n\nThis file is daemon-managed. Update node instructions through workflow configuration tooling.\n\nNode: {node_id}\n"
                )
            });
        if let Err(error) = std::fs::write(&path, content) {
            tracing::debug!(
                ?error,
                "Failed to write workflow instruction file at {:?}",
                path
            );
            return None;
        }
    }
    Some(path.to_string_lossy().to_string())
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
        .get_run_for_agent(session_id, node_run.agent_id())
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
    let history = match app.session_history(session_id) {
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
    let started_at_ms = node_run
        .started_at_ms()
        .unwrap_or_else(|| node_run.created_at_ms());
    let provider_output = history
        .into_iter()
        .filter(|entry| {
            entry.provider_run_id.as_deref() == Some(provider_run_id.as_str())
                && entry.timestamp_ms >= started_at_ms
                && entry.kind == SessionHistoryEntryKind::ProviderOutput
        })
        .map(|entry| entry.text)
        .collect::<Vec<_>>()
        .join("");
    let structured_output = parse_workflow_structured_output(&provider_output);
    if structured_output.is_none() {
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

fn serialize_handoff_payloads_json(handoff_messages: Option<&[WorkflowMessage]>) -> Option<String> {
    let handoff_payloads = handoff_messages
        .map(|messages| {
            messages
                .iter()
                .map(|message| {
                    serde_json::from_str::<serde_json::Value>(message.handoff_payload())
                        .unwrap_or_else(|_| {
                            serde_json::Value::String(message.handoff_payload().to_string())
                        })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if handoff_payloads.is_empty() {
        None
    } else {
        serde_json::to_string_pretty(&handoff_payloads).ok()
    }
}

fn workflow_turn_delivery_token(workflow_node_run_id: &str) -> String {
    format!("workflow-ack:{workflow_node_run_id}")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use crate::attachment::{AttachRequest, ClientCapabilityLevel};
    use crate::local::{
        AddWorkflowNodeRequest, CreateWorkflowEndpointRequest, CreateWorkflowRequest,
        InvokeWorkflowEndpointRequest, LocalDaemonRequest, SpawnAgentRequest,
        UpdateWorkflowNodeInstructionsRequest,
    };
    use crate::provider::LaunchProviderRequest;
    use crate::session::{CreateSessionRequest, WorkflowMessage};
    use crate::{DaemonApp, DaemonConfig};

    use super::{parse_workflow_structured_output, prepare_workflow_turn_prompt};

    #[test]
    fn workflow_instruction_reference_is_written_under_agent_workdir() {
        let mut app = DaemonApp::bootstrap(DaemonConfig::for_tests()).expect("daemon should boot");
        let (session, _default_agent) = match app
            .handle_local_request(LocalDaemonRequest::CreateSession(
                CreateSessionRequest::new("workspace-scheduler", "worktree-scheduler"),
            ))
            .expect("session should exist")
        {
            crate::local::LocalDaemonResponse::SessionCreated { session, agent } => {
                (session, agent)
            }
            other => panic!("unexpected response: {other:?}"),
        };
        app.attach(AttachRequest::new(
            session.id(),
            "client-scheduler",
            ClientCapabilityLevel::InteractiveStructured,
        ))
        .expect("attachment should attach");
        let agent_id = match app
            .handle_local_request(LocalDaemonRequest::SpawnAgent(SpawnAgentRequest {
                session_id: session.id().to_string(),
                alias: Some("agent-scheduler".to_string()),
                provider: "dev-stub".to_string(),
                model: Some("test-model".to_string()),
                effort: None,
                worktree_id: Some("worktree-scheduler".to_string()),
            }))
            .expect("agent should spawn")
        {
            crate::local::LocalDaemonResponse::AgentSpawned { agent } => agent.id().to_string(),
            other => panic!("unexpected response: {other:?}"),
        };

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

        let workflow_id = match app
            .handle_local_request(LocalDaemonRequest::CreateWorkflow(CreateWorkflowRequest {
                session_id: session.id().to_string(),
                alias: Some("wf-scheduler".to_string()),
            }))
            .expect("workflow should exist")
        {
            crate::local::LocalDaemonResponse::WorkflowCreated { workflow, .. } => {
                workflow.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        let node_id = match app
            .handle_local_request(LocalDaemonRequest::AddWorkflowNode(
                AddWorkflowNodeRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id.clone(),
                    agent_id: agent_id.clone(),
                },
            ))
            .expect("node should be added")
        {
            crate::local::LocalDaemonResponse::WorkflowNodeAdded { node, .. } => {
                node.id().to_string()
            }
            other => panic!("unexpected response: {other:?}"),
        };
        app.handle_local_request(LocalDaemonRequest::UpdateWorkflowNodeInstructions(
            UpdateWorkflowNodeInstructionsRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                node_id: node_id.clone(),
                instructions: Some("Read me from a workspace-local hidden file.".to_string()),
            },
        ))
        .expect("instructions should update");
        app.handle_local_request(LocalDaemonRequest::CreateWorkflowEndpoint(
            CreateWorkflowEndpointRequest {
                session_id: session.id().to_string(),
                workflow_ref: workflow_id.clone(),
                entry_node_id: node_id.clone(),
                alias: Some("entry".to_string()),
            },
        ))
        .expect("endpoint should exist");
        let workflow_run = match app
            .handle_local_request(LocalDaemonRequest::InvokeWorkflowEndpoint(
                InvokeWorkflowEndpointRequest {
                    session_id: session.id().to_string(),
                    workflow_ref: workflow_id,
                    endpoint_ref: "entry".to_string(),
                    prompt: Some("start".to_string()),
                },
            ))
            .expect("workflow should invoke")
        {
            crate::local::LocalDaemonResponse::WorkflowRunInvoked { workflow_run, .. } => {
                workflow_run
            }
            other => panic!("unexpected response: {other:?}"),
        };
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
        assert!(prompt.contains("If the node instruction reference is present"));
        let _ = fs::remove_dir_all(PathBuf::from(workdir));
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

fn default_workflow_system_prompt_template() -> &'static str {
    "Before producing substantive output, call the Arroba runtime MCP tool `ack_workflow_turn` exactly once with this JSON argument object:\n{\"delivery_token\":\"{{DELIVERY_TOKEN}}\"}\n\nThis acknowledgment is for runtime delivery tracking and is separate from the final validated workflow output. Do not describe the acknowledgment in your final answer.\n\nA shared workflow console is available through the Arroba runtime MCP tools `workflow_console_read`, `workflow_console_write`, and `workflow_console_clear`. Use those tools only if your node instruction file requires shared console output or inspection.\n\n{{WORKFLOW_HANDOFF_PAYLOADS_BLOCK}}{{OUTGOING_EDGE_CONTRACTS_BLOCK}}{{NODE_INSTRUCTION_REFERENCE_BLOCK}}If the node instruction reference is present and you do not remember the instructions exactly, read that markdown file before finalizing the turn.\n\n{{CONTROL_MAILBOX_BLOCK}}At the end of this workflow turn, return exactly one fenced ```json block with this shape:\n{\"summary\":\"human-facing summary\",\"output\":{\"message\":\"explicit downstream output message\"}}\nDo not output any prose before or after that fenced block. Do not mention acknowledgments, tool calls, or workflow mechanics in the summary unless the task explicitly requires it. The downstream payload is only output.message plus any workflow-owned artifacts.\n\nIf a Control mailbox is present, resolve every listed issue before finalizing and do not repeat the invalid payload. If a handoff payload or outgoing edge contract includes output_schema_ref, call the Arroba runtime MCP tool `validate_workflow_output` before finalizing. Pass the same delivery_token that was provided for `ack_workflow_turn`, and validate your proposed output.message JSON against that schema ref.\n\nValidation is a gate, not a suggestion. If `validate_workflow_output` returns `valid: false` or any warning, do not finalize the turn yet. Revise the proposed output, call `validate_workflow_output` again, and only finalize once the tool returns `valid: true` with no warning. A single failed validation call does not satisfy this turn's completion requirements."
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
