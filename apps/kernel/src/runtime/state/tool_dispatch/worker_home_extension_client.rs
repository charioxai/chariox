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
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        self.register_remote_home_extension_invocation(&context, &metadata)
            .await;
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &relay_config,
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
        )
        .await;
        self.unregister_remote_home_extension_invocation(&context, &metadata)
            .await;
        let response = response?;
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
        if let Some(tool) = run
            .remote_extension_manifest()
            .home_proxy_tool(name)
            .filter(|tool| tool.kind == crate::extension::ExtensionKind::Mcp)
            .cloned()
        {
            return self
                .dispatch_remote_home_mcp_proxy_call(&run, name, tool, payload)
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
        tool: crate::extension::RemoteExtensionTool,
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
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        self.register_remote_home_extension_invocation(&context, &metadata)
            .await;
        let response = crate::transport::relay_client::send_peer_request_via_temporary_connection(
            &relay_config,
            ClientTarget {
                daemon_id: Some(home_kernel_id.clone()),
                daemon_alias: None,
            },
            RelayPeerRequest::InvokeHomeMcpProxy {
                context: context.clone(),
                metadata: metadata.clone(),
                name: name.to_string(),
                tool,
                payload: payload.clone(),
            },
        )
        .await;
        self.unregister_remote_home_extension_invocation(&context, &metadata)
            .await;
        let response = response?;
        match response {
            RelayPeerResponse::HomeMcpProxyHandled { response } => Ok(response),
            other => Err(DaemonError::LocalTransport {
                operation: "remote MCP proxy",
                message: format!("unexpected home MCP proxy response: {other:?}"),
            }),
        }
    }

    async fn register_remote_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) {
        self.owned
            .remote_home_extension_inflight
            .lock()
            .await
            .entry(context.leased_agent_id.clone())
            .or_default()
            .push(
                crate::runtime::state::RemoteHomeExtensionInflightInvocation {
                    context: context.clone(),
                    metadata: metadata.clone(),
                },
            );
    }

    async fn unregister_remote_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) {
        let mut inflight = self.owned.remote_home_extension_inflight.lock().await;
        let Some(entries) = inflight.get_mut(&context.leased_agent_id) else {
            return;
        };
        entries.retain(|entry| entry.metadata.invocation_id != metadata.invocation_id);
        if entries.is_empty() {
            inflight.remove(&context.leased_agent_id);
        }
    }

    pub(crate) async fn cancel_remote_home_extension_invocations_for_leased_agent(
        &self,
        leased_agent_id: &str,
    ) {
        let entries = self
            .owned
            .remote_home_extension_inflight
            .lock()
            .await
            .remove(leased_agent_id)
            .unwrap_or_default();
        if entries.is_empty() {
            return;
        }
        let relay_config = self.with_app_side_effect(|app| app.config().clone()).await;
        for entry in entries {
            let _ = crate::transport::relay_client::send_peer_request_via_temporary_connection(
                &relay_config,
                ClientTarget {
                    daemon_id: Some(entry.context.home_kernel_id.clone()),
                    daemon_alias: None,
                },
                RelayPeerRequest::CancelHomeExtensionInvocation {
                    context: entry.context,
                    metadata: entry.metadata,
                },
            )
            .await;
        }
    }
}
