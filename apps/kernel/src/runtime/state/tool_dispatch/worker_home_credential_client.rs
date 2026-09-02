use chariox_relay::protocol::ClientTarget;
use std::time::Duration;

use super::*;

const REMOTE_CREDENTIAL_PROMPT_RESPONSE_BUFFER: Duration = Duration::from_secs(15);

impl KernelRuntimeState {
    pub(super) async fn try_dispatch_remote_home_credential_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        if tool_name == crate::transport::runtime_tools::REQUEST_POPUP_TOOL {
            return Ok(None);
        }
        if tool_name == crate::transport::runtime_tools::SEND_SECRET_TO_TERMINAL_TOOL {
            return self
                .try_dispatch_remote_home_terminal_secret_input(provider_run, arguments)
                .await;
        }
        let Some(context) = self
            .remote_credential_context_for_provider_run(provider_run)
            .await
        else {
            return Ok(None);
        };
        let home_kernel_id = context.home_kernel_id.clone();
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        let response_timeout =
            remote_home_credential_tool_response_timeout(&relay_config, tool_name, &arguments);
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection_with_timeout(
            &relay_config,
            ClientTarget {
                daemon_id: Some(home_kernel_id),
                daemon_alias: None,
            },
            RelayPeerRequest::InvokeHomeCredentialTool {
                context,
                tool_name: tool_name.to_string(),
                arguments,
            },
            response_timeout,
        )
        .await?;
        match response {
            RelayPeerResponse::HomeCredentialToolHandled { result } => Ok(Some(result)),
            other => Err(DaemonError::LocalTransport {
                operation: "remote credential proxy",
                message: format!("unexpected home credential response: {other:?}"),
            }),
        }
    }

    pub(super) async fn resolve_remote_home_credential_secret(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        credential_id: &str,
        injection: crate::transport::relay_peer::RemoteCredentialSecretInjection,
    ) -> Result<Option<zeroize::Zeroizing<String>>, DaemonError> {
        let Some(context) = self
            .remote_credential_context_for_provider_run(provider_run)
            .await
        else {
            return Ok(None);
        };
        let home_kernel_id = context.home_kernel_id.clone();
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(home_kernel_id),
                daemon_alias: None,
            },
            RelayPeerRequest::ResolveHomeCredentialSecret {
                context,
                credential_id: credential_id.to_string(),
                injection,
            },
        )
        .await?;
        match response {
            RelayPeerResponse::HomeCredentialSecretResolved { secret_input, .. } => {
                Ok(Some(secret_input.into_zeroizing()))
            }
            other => Err(DaemonError::LocalTransport {
                operation: "remote credential secret",
                message: format!("unexpected home credential secret response: {other:?}"),
            }),
        }
    }

    async fn try_dispatch_remote_home_terminal_secret_input(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::SendSecretToTerminalArgs,
        >(arguments)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_send_secret_to_terminal",
            message: format!("invalid tool arguments: {error}"),
        })?;
        let Some(mut input) = self
            .resolve_remote_home_credential_secret(
                provider_run,
                &args.credential_id,
                crate::transport::relay_peer::RemoteCredentialSecretInjection::Pty,
            )
            .await?
        else {
            return Ok(None);
        };
        if args.append_newline {
            input.push('\n');
        }
        let provider_run_id = provider_run.id().to_string();
        self.with_app_side_effect(move |app| {
            app.write_provider_pty_input_for_runtime(&provider_run_id, input.as_bytes())
        })
        .await?;
        Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
            ok: true,
            payload: serde_json::json!({
                "submitted": true,
                "credential_id": args.credential_id,
                "target": "current_provider_run",
            }),
        }))
    }

    async fn remote_credential_context_for_provider_run(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
    ) -> Option<crate::transport::relay_peer::RemoteExtensionInvocationContext> {
        self.with_app_side_effect(|app| {
            crate::app::RemoteLeaseRuntime::new(app)
                .leased_extension_invocation_context_for_runtime_provider_run(provider_run)
        })
        .await
    }
}

fn remote_home_credential_tool_response_timeout(
    config: &crate::config::DaemonConfig,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Duration {
    let default_timeout = Duration::from_millis(config.relay_request_timeout_ms);
    if tool_name != crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL {
        return default_timeout;
    }
    let Some(timeout_sec) = arguments
        .get("prompt")
        .and_then(|prompt| prompt.get("timeout_sec"))
        .and_then(serde_json::Value::as_u64)
    else {
        return default_timeout;
    };
    std::cmp::max(
        default_timeout,
        Duration::from_secs(timeout_sec).saturating_add(REMOTE_CREDENTIAL_PROMPT_RESPONSE_BUFFER),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_timeout(timeout_ms: u64) -> crate::config::DaemonConfig {
        let mut config = crate::config::DaemonConfig::for_tests();
        config.relay_request_timeout_ms = timeout_ms;
        config
    }

    #[test]
    fn request_credential_secret_uses_prompt_timeout_for_relay_response() {
        let config = config_with_timeout(60_000);
        let timeout = remote_home_credential_tool_response_timeout(
            &config,
            crate::transport::runtime_tools::REQUEST_CREDENTIAL_SECRET_TOOL,
            &serde_json::json!({
                "prompt": {
                    "timeout_sec": 600
                }
            }),
        );
        assert_eq!(timeout, Duration::from_secs(615));
    }

    #[test]
    fn other_home_credential_tools_keep_default_relay_timeout() {
        let config = config_with_timeout(60_000);
        let timeout = remote_home_credential_tool_response_timeout(
            &config,
            crate::transport::runtime_tools::LIST_CREDENTIAL_HANDLES_TOOL,
            &serde_json::json!({
                "prompt": {
                    "timeout_sec": 600
                }
            }),
        );
        assert_eq!(timeout, Duration::from_secs(60));
    }
}
