use arroba_relay::protocol::ClientTarget;

use super::*;

impl KernelRuntimeState {
    pub(super) async fn try_dispatch_remote_home_extension_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let Some(tool) = provider_run
            .remote_extension_manifest()
            .home_proxy_tool(tool_name)
            .filter(|tool| {
                matches!(
                    tool.kind,
                    crate::extension::ExtensionKind::Script
                        | crate::extension::ExtensionKind::Connector
                )
            })
            .cloned()
        else {
            return Ok(None);
        };
        let context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_extension_invocation_context_for_provider_run(provider_run.id())
            })
            .await
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "remote extension proxy",
                message: "provider run is not attached to a remote execution lease".to_string(),
            })?;
        let home_kernel_id = context.home_kernel_id.clone();
        let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
            provider_run.id(),
            tool_name,
            None,
        );
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::InvokeHomeExtensionTool {
                            context: context.clone(),
                            metadata: metadata.clone(),
                            tool: tool.clone(),
                            arguments: arguments.clone(),
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::HomeExtensionToolHandled { result } => Ok(Some(result)),
            other => Err(DaemonError::LocalTransport {
                operation: "remote extension proxy",
                message: format!("unexpected home extension response: {other:?}"),
            }),
        }
    }

    pub(crate) async fn dispatch_authenticated_mcp_proxy_call(
        &self,
        provider_run_projection: &crate::runtime::projection::ProviderRunProjectionStore,
        auth_token: &str,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        crate::mcp::validate_registry_name(name, "mcp name")?;
        let run = provider_run_projection
            .get_by_runtime_mcp_auth_token(auth_token)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "mcp.proxy.auth",
                message: "invalid runtime MCP auth token".to_string(),
            })?;
        if run
            .remote_extension_manifest()
            .home_proxy_tool(name)
            .is_some_and(|tool| tool.kind == crate::extension::ExtensionKind::Mcp)
        {
            return self
                .dispatch_remote_home_mcp_proxy_call(&run, name, payload)
                .await;
        }
        if run.mcp_servers().iter().any(|server| {
            server.name == name
                && matches!(
                    &server.transport,
                    crate::mcp::ArrobaMcpTransportConfig::StreamableHttp { url, .. }
                        if url == "http://127.0.0.1/mcp"
                )
        }) {
            return Err(DaemonError::LocalTransport {
                operation: "remote MCP proxy",
                message: format!("home-proxy MCP `{name}` is no longer granted"),
            });
        }
        crate::runtime::runtime_mcp_proxy_dispatcher::dispatch_authenticated_mcp_proxy_call(
            provider_run_projection,
            auth_token,
            name,
            payload,
        )
        .await
    }

    async fn dispatch_remote_home_mcp_proxy_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app)
                    .leased_extension_invocation_context_for_provider_run(provider_run.id())
            })
            .await
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "remote MCP proxy",
                message: "provider run is not attached to a remote execution lease".to_string(),
            })?;
        let home_kernel_id = context.home_kernel_id.clone();
        let provider_tool_call_id = payload.get("id").and_then(|value| match value {
            serde_json::Value::String(value) => Some(value.clone()),
            serde_json::Value::Number(value) => Some(value.to_string()),
            _ => None,
        });
        let metadata = crate::extension::RemoteExtensionInvocationMetadata::new(
            provider_run.id(),
            name,
            provider_tool_call_id,
        );
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::InvokeHomeMcpProxy {
                            context: context.clone(),
                            metadata: metadata.clone(),
                            name: name.to_string(),
                            payload: payload.clone(),
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::HomeMcpProxyHandled { response } => Ok(response),
            other => Err(DaemonError::LocalTransport {
                operation: "remote MCP proxy",
                message: format!("unexpected home MCP proxy response: {other:?}"),
            }),
        }
    }
}
