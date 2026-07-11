//! Relay worker submission and stale remote-agent binding refresh for remote prompts.

use super::*;

pub(super) async fn submit_remote_prompt_to_worker_with_binding_refresh(
    state: &KernelRuntimeState,
    dispatch: &mut crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
) -> Result<String, DaemonError> {
    let result = submit_remote_prompt_to_worker(
        state,
        dispatch,
        prompt.clone(),
        attachments.clone(),
        required_mcps.clone(),
        required_skills.clone(),
        remote_extension_manifest.clone(),
        "unexpected remote prompt response",
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
        required_skills,
        remote_extension_manifest,
        "unexpected remote prompt response after binding refresh",
    )
    .await
}

async fn submit_remote_prompt_to_worker(
    state: &KernelRuntimeState,
    dispatch: &crate::app::KernelRemotePromptDispatch,
    prompt: String,
    attachments: Vec<crate::transport::relay_peer::RelayPromptAttachment>,
    required_mcps: Vec<crate::transport::relay_peer::RequiredRemoteMcp>,
    required_skills: Option<Vec<crate::transport::relay_peer::RequiredRemoteSkill>>,
    remote_extension_manifest: crate::extension::RemoteExtensionManifest,
    unexpected_response_message: &'static str,
) -> Result<String, DaemonError> {
    let config = remote_dispatch_relay_config(state.config_snapshot().await, dispatch);
    match crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
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
            required_skills,
            remote_extension_manifest,
        },
        crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT,
    )
    .await
    {
        Ok(RelayPeerResponse::LeasedPromptSubmitted {
            provider_run_id, ..
        }) => Ok(provider_run_id),
        Ok(other) => Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: format!("{unexpected_response_message}: {other:?}"),
        }),
        Err(error) => Err(error),
    }
}

fn remote_prompt_dispatch_should_refresh_binding(result: &Result<String, DaemonError>) -> bool {
    let Err(error) = result else {
        return false;
    };
    remote_prompt_error_should_refresh_binding(error)
}

pub(super) fn remote_prompt_error_should_refresh_binding(error: &DaemonError) -> bool {
    match error {
        DaemonError::LeasedAgentNotFound { .. } | DaemonError::ExecutionLeaseNotFound { .. } => {
            true
        }
        DaemonError::LocalTransport { message, .. } => {
            message.contains("leased agent") && message.contains("was not found")
                || message.contains("execution lease") && message.contains("was not found")
                || message.contains("leased_agent_not_found")
                || message.contains("execution_lease_not_found")
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
        source_attachment_id: Some(dispatch.source_attachment_id.clone()),
        workspace_live_sync_mode: dispatch.workspace_live_sync_mode,
        prompt_origin: Some(dispatch.prompt_origin),
        external_provider: dispatch.external_provider.clone(),
        external_provider_session_id: dispatch.external_provider_session_id.clone(),
        external_provider_turn_id: dispatch.external_provider_turn_id.clone(),
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
        config.apply_remote_relay_override(relay_url, relay_token);
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_prompt_dispatch_does_not_refresh_binding_after_worker_timeout() {
        let result = Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "remote prompt dispatch timed out waiting for worker response".to_string(),
        });

        assert!(!remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn remote_prompt_dispatch_refreshes_binding_for_missing_lease_errors() {
        let result = Err(DaemonError::LocalTransport {
            operation: "submit remote prepared prompt",
            message: "leased_agent_not_found".to_string(),
        });

        assert!(remote_prompt_dispatch_should_refresh_binding(&result));
    }

    #[test]
    fn leased_prompt_submit_timeout_covers_codex_mcp_retry_window() {
        assert!(
            crate::transport::relay_client::LEASED_PROMPT_SUBMIT_RESPONSE_TIMEOUT
                > std::time::Duration::from_secs(180)
        );
    }
}
