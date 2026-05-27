use std::path::PathBuf;

use arroba_relay::protocol::ClientTarget;

use crate::app::DaemonApp;
use crate::error::DaemonError;
use crate::provider::LaunchProviderRequest;
use crate::session::{PromptQueueItem, PromptSubmissionOutcome};
use crate::transport::relay_client::send_peer_request_via_temporary_connection;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

pub(super) fn submit_claimed_workflow_prompt(
    app: &mut DaemonApp,
    session_id: &str,
    workflow_run_id: &str,
    workflow_node_run_id: &str,
    target_agent_id: &str,
    prompt: &str,
) -> Result<PromptSubmissionOutcome, DaemonError> {
    let outcome = app.prompt_owner_submit_workflow_prompt(
        session_id,
        &super::workflow_prompt_source_attachment_id(workflow_run_id),
        target_agent_id,
        workflow_run_id,
        workflow_node_run_id,
        prompt.to_string(),
    )?;
    Ok(outcome)
}

pub(super) fn dispatch_workflow_prompt(
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

pub(super) fn ensure_workflow_provider_run_for_agent(
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
                if agent.remote_execution().is_some() {
                    return Err(DaemonError::LocalTransport {
                        operation: "ensure workflow provider run for agent",
                        message: format!(
                            "agent `{agent_id}` is remote-backed and must relaunch its provider on the worker kernel"
                        ),
                    });
                }
                let adapter_key = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let provider = match agent.provider() {
                    "default" => "opencode",
                    value => value,
                };
                let session = app.sessions().get_session(session_id)?;
                let effective_config =
                    crate::session::effective_agent_execution_config(&session, Some(&agent));
                let mut request = LaunchProviderRequest::new(
                    session_id,
                    adapter_key,
                    provider,
                    "default",
                    agent.model().unwrap_or("default"),
                )
                .with_agent_id(agent.id().to_string())
                .with_variant(agent.effort().map(str::to_string))
                .with_execution_mode(effective_config.mode)
                .with_permission_level(effective_config.permission_level);
                if crate::provider::provider_requires_workspace_live_sync_by_default(provider, app.config())
                {
                    request = request.with_workspace_live_sync_required();
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
            if crate::provider::provider_requires_workspace_live_sync_by_default(provider, app.config()) {
                request = request.with_workspace_live_sync_required();
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
