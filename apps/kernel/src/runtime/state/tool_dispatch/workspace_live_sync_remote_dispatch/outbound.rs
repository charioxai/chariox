//! Outbound workspace live sync forwarding from a leased worker to the home kernel.

use super::super::*;

const REMOTE_WORKSPACE_LIVE_SYNC_RECOVERY_WINDOW: Duration = Duration::from_secs(240);
const REMOTE_WORKSPACE_LIVE_SYNC_RETRY_DELAY: Duration = Duration::from_millis(250);

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn try_dispatch_remote_workspace_live_sync_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .workspace_live_sync_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_workspace_live_sync_context_for_provider_run(
                        provider_run.id(),
                        workspace_context.identity.clone(),
                    )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(workspace_live_sync_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let artifact_states = remote_workspace_live_sync_artifact_states_for_tool(
            &workspace_context.root,
            tool_name,
            &arguments,
        )?;
        let metadata = crate::transport::relay_peer::RemoteWorkspaceLiveSyncInvocationMetadata::new(
            provider_run.id(),
            tool_name,
            None,
        );
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        let response = send_remote_workspace_live_sync_request_with_recovery(
            &relay_config,
            ClientTarget {
                daemon_id: Some(remote_context.home_kernel_id.clone()),
                daemon_alias: None,
            },
            |attempt| {
                let mut attempt_metadata = metadata.clone();
                attempt_metadata.attempt = attempt;
                RelayPeerRequest::ForwardWorkspaceLiveSyncRuntimeTool {
                    context: remote_context.clone(),
                    metadata: attempt_metadata,
                    tool_name: tool_name.to_string(),
                    arguments: arguments.clone(),
                    artifact_states: artifact_states.clone(),
                }
            },
        )
        .await?;
        let (mut result, final_states) = match response {
            RelayPeerResponse::WorkspaceLiveSyncRuntimeToolHandled {
                result,
                final_artifact_states,
            } => (result, final_artifact_states),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased workspace live sync runtime tool",
                    message: format!(
                        "unexpected forwarded workspace live sync response: {other:?}"
                    ),
                });
            }
        };
        if result.ok && !final_states.is_empty() {
            if let Some(rejection) = apply_remote_workspace_live_sync_final_states(
                &workspace_context.root,
                &artifact_states,
                &final_states,
            )? {
                result = rejection;
            } else {
                let finalize_response = send_remote_workspace_live_sync_request_with_recovery(
                    &relay_config,
                    ClientTarget {
                        daemon_id: Some(remote_context.home_kernel_id.clone()),
                        daemon_alias: None,
                    },
                    |attempt| {
                        let mut attempt_metadata = metadata.clone();
                        attempt_metadata.attempt = attempt;
                        RelayPeerRequest::FinalizeWorkspaceLiveSyncRuntimeTool {
                            context: remote_context.clone(),
                            metadata: attempt_metadata,
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                            initial_artifact_states: artifact_states.clone(),
                            final_artifact_states: final_states.clone(),
                        }
                    },
                )
                .await?;
                if !matches!(
                    finalize_response,
                    RelayPeerResponse::WorkspaceLiveSyncRuntimeToolFinalized
                ) {
                    return Err(DaemonError::LocalTransport {
                        operation: "finalize leased workspace live sync runtime tool",
                        message: format!(
                            "unexpected forwarded workspace live sync finalize response: {finalize_response:?}"
                        ),
                    });
                }
            }
        }
        add_workspace_live_sync_workspace_payload(&mut result.payload, &workspace_context);
        Ok(Some(result))
    }
}

async fn send_remote_workspace_live_sync_request_with_recovery(
    config: &crate::config::DaemonConfig,
    target: ClientTarget,
    mut request_for_attempt: impl FnMut(u32) -> RelayPeerRequest,
) -> Result<RelayPeerResponse, DaemonError> {
    let started = Instant::now();
    let mut attempt = 1_u32;
    loop {
        let remaining =
            REMOTE_WORKSPACE_LIVE_SYNC_RECOVERY_WINDOW.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(DaemonError::LocalTransport {
                operation: "recover forwarded workspace live sync request",
                message: format!(
                    "relay recovery window expired after {}ms",
                    REMOTE_WORKSPACE_LIVE_SYNC_RECOVERY_WINDOW.as_millis()
                ),
            });
        }
        let response_timeout =
            Duration::from_millis(config.relay_request_timeout_ms).min(remaining);
        match crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            config,
            target.clone(),
            request_for_attempt(attempt),
            response_timeout,
        )
        .await
        {
            Ok(response) => return Ok(response),
            Err(error)
                if remote_workspace_live_sync_relay_error_is_retryable(&error)
                    && started.elapsed() < REMOTE_WORKSPACE_LIVE_SYNC_RECOVERY_WINDOW =>
            {
                crate::logging::warn_with_fields(
                    "daemon.workspace_live_sync",
                    "retrying forwarded workspace live sync request after relay interruption",
                    serde_json::json!({
                        "attempt": attempt,
                        "elapsed_ms": started.elapsed().as_millis(),
                        "error": error.to_string(),
                    }),
                );
                let delay = REMOTE_WORKSPACE_LIVE_SYNC_RETRY_DELAY.min(
                    REMOTE_WORKSPACE_LIVE_SYNC_RECOVERY_WINDOW
                        .saturating_sub(started.elapsed()),
                );
                tokio::time::sleep(delay).await;
                attempt = attempt.saturating_add(1);
            }
            Err(error) => return Err(error),
        }
    }
}

fn remote_workspace_live_sync_relay_error_is_retryable(error: &DaemonError) -> bool {
    let DaemonError::LocalTransport { operation, message } = error else {
        return false;
    };
    if matches!(
        *operation,
        "connect temporary relay peer socket"
            | "write temporary relay register"
            | "write temporary relay peer request"
    ) {
        return true;
    }
    let message = message.to_ascii_lowercase();
    let transient_message = [
        "timed out",
        "timeout",
        "connection",
        "not connected",
        "not currently visible",
        "did not appear",
        "closed",
        "reset",
        "refused",
        "broken pipe",
        "temporarily unavailable",
        "websocket",
    ]
    .iter()
    .any(|candidate| message.contains(candidate));
    transient_message
        && matches!(
            *operation,
            "read temporary relay peer response"
                | "get_live_kernel"
                | "relay_metadata_query"
                | "connect relay metadata socket"
                | "write relay metadata request"
                | "read relay metadata response"
        )
}

#[cfg(test)]
mod recovery_tests {
    use super::*;

    #[test]
    fn retries_transport_interruptions_without_retrying_home_kernel_rejections() {
        let timeout = DaemonError::LocalTransport {
            operation: "read temporary relay peer response",
            message: "timed out waiting for relay peer response after 60000ms".to_string(),
        };
        let disconnected = DaemonError::LocalTransport {
            operation: "get_live_kernel",
            message: "kernel `home` is not currently visible on relay".to_string(),
        };
        let rejected = DaemonError::LocalTransport {
            operation: "read temporary relay peer response",
            message: "workspace identity does not match the home session".to_string(),
        };

        assert!(remote_workspace_live_sync_relay_error_is_retryable(
            &timeout
        ));
        assert!(remote_workspace_live_sync_relay_error_is_retryable(
            &disconnected
        ));
        assert!(!remote_workspace_live_sync_relay_error_is_retryable(
            &rejected
        ));
    }
}
