//! Relay worker submission and stale remote-agent binding refresh for remote prompts.

use super::*;

pub(super) async fn submit_remote_prompt_to_worker_with_binding_refresh(
    state: &KernelRuntimeState,
    dispatch: &mut crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
) -> Result<String, DaemonError> {
    let result = submit_remote_prompt_to_worker(
        state,
        dispatch,
        prompt.clone(),
        attachments.clone(),
        required_mcps.clone(),
        "unexpected remote prompt response",
        "remote prompt dispatch timed out waiting for worker response",
    )
    .await;
    if !remote_prompt_dispatch_should_refresh_binding(&result) {
        return result;
    }

    crate::logging::warn_with_fields(
        "daemon.remote_prompt_dispatch",
        "remote prompt lease stale; refreshing binding",
        serde_json::json!({
            "session_id": dispatch.session_id,
            "agent_id": dispatch.agent_id,
            "worker_kernel_id": dispatch.worker_kernel_id,
            "leased_agent_id": dispatch.leased_agent_id,
        }),
    );
    let agent = state
        .with_app_side_effect(|app| app.refresh_remote_agent_binding(&dispatch.agent_id))
        .await?;
    let Some(remote_execution) = agent.remote_execution().cloned() else {
        return Err(DaemonError::LocalTransport {
            operation: "refresh remote prompt binding",
            message: format!(
                "agent `{}` did not have remote execution after binding refresh",
                dispatch.agent_id
            ),
        });
    };
    dispatch.worker_kernel_id = remote_execution.worker_kernel_id;
    dispatch.leased_agent_id = remote_execution.leased_agent_id;
    dispatch.relay_url = remote_execution.relay_url;
    dispatch.relay_token = remote_execution.relay_token;
    submit_remote_prompt_to_worker(
        state,
        dispatch,
        prompt,
        attachments,
        required_mcps,
        "unexpected remote prompt response after binding refresh",
        "remote prompt dispatch timed out after binding refresh",
    )
    .await
}

async fn submit_remote_prompt_to_worker(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    unexpected_response_message: &'static str,
    timeout_message: &'static str,
) -> Result<String, DaemonError> {
    let config = remote_dispatch_relay_config(state.config_snapshot().await, dispatch);
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &config,
            ClientTarget {
                daemon_id: Some(dispatch.worker_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::SubmitLeasedPrompt {
                leased_agent_id: dispatch.leased_agent_id.clone(),
                prompt,
                attachments,
                workflow_context: dispatch.workflow_context.clone(),
                git_context: Some(remote_git_turn_context(dispatch)),
                required_mcps,
            },
        ),
    )
    .await
    {
        Ok(Ok(RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id, ..
        })) => Ok(provider_run_id),
        Ok(Ok(other)) => Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: format!("{unexpected_response_message}: {other:?}"),
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: timeout_message.to_string(),
        }),
    }
}

fn remote_prompt_dispatch_should_refresh_binding(result: &Result<String, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    match error {
        DaemonError::LeasedAgentNotFound { .. } | DaemonError::ExecutionLeaseNotFound { .. } => {
            true
        }
        DaemonError::LocalTransport { message, .. } => {
            message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
                || message.contains("leased_agent_not_found")
                || message.contains("execution_lease_not_found")
                || message.contains("timed out waiting for worker response")
        }
        _ => false,
    }
}

fn remote_git_turn_context(
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::transport::relay_peer::RemoteGitTurnContext {
    crate::transport::relay_peer::RemoteGitTurnContext {
        home_session_id: dispatch.session_id.clone(),
        home_agent_id: dispatch.agent_id.clone(),
        home_prompt_id: dispatch.prompt_id.clone(),
        home_turn_id: dispatch.prompt_id.clone(),
        prompt_summary: crate::prompt_transcript::render_prompt_transcript(
            &dispatch.prompt,
            &dispatch.attachments,
        ),
    }
}

fn remote_dispatch_relay_config(
    mut config: crate::config::DaemonConfig,
    dispatch: &crate::app::KernelRemotePromptDispatch,
) -> crate::config::DaemonConfig {
    if let (Some(relay_url), Some(relay_token)) =
        (dispatch.relay_url.clone(), dispatch.relay_token.clone())
    {
        config.relay_url = Some(relay_url);
        config.relay_token = Some(relay_token);
        config.cloud_relay = None;
    }
    config
}
