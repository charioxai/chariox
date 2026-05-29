use arroba_relay::protocol::ClientTarget;

use super::capability_registry::{
    connector_adapter_registry, connector_registry, environment_registry_for_workspace,
    mcp_registry_for_workspace, script_registry_for_workspace,
};
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

    pub(crate) async fn dispatch_forwarded_home_extension_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.validate_home_extension_invocation(&context, &tool)?;
        match tool.kind {
            crate::extension::ExtensionKind::Script => {
                self.dispatch_home_script_tool(&context, &tool, arguments)
            }
            crate::extension::ExtensionKind::Connector => {
                self.dispatch_home_connector_tool(&context, &tool, arguments)
                    .await
            }
            _ => Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "home extension runtime tool invocation only supports scripts and connectors"
                }),
            }),
        }
    }

    pub(crate) async fn dispatch_forwarded_home_mcp_proxy_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        name: String,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let tool = crate::extension::RemoteExtensionTool {
            kind: crate::extension::ExtensionKind::Mcp,
            name: name.clone(),
            tool_name: name.clone(),
            description: String::new(),
            input_schema: serde_json::json!({}),
            authority: crate::extension::ExtensionAuthority::Home,
            definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
            execution_location: crate::extension::ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: None,
            version_hash: None,
        };
        self.validate_home_extension_invocation(&context, &tool)?;
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let registry = mcp_registry_for_workspace(session.workspace_id());
        let Some(config) = registry.get(&name)? else {
            return Err(DaemonError::LocalTransport {
                operation: "home MCP proxy",
                message: format!("MCP `{name}` is granted but is not installed on home"),
            });
        };
        tokio::task::spawn_blocking(move || {
            crate::provider::dispatch_provider_mcp_proxy_request(&config, payload)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "home MCP proxy",
            message: error.to_string(),
        })?
    }

    fn validate_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let config = self.owned.config_projection.snapshot();
        if config.daemon_id != context.home_kernel_id {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "invocation was sent to the wrong home kernel".to_string(),
            });
        }
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        if agent.session_id() != context.home_session_id {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "agent does not belong to invocation session".to_string(),
            });
        }
        let Some(remote_execution) = agent.remote_execution() else {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "agent is not remote-backed".to_string(),
            });
        };
        if remote_execution.leased_agent_id != context.leased_agent_id {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "leased agent does not match home agent binding".to_string(),
            });
        }
        if !agent.has_extension_grant(tool.kind.clone(), &tool.name) {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: format!(
                    "extension `{}:{}` is not granted",
                    tool.kind.as_str(),
                    tool.name
                ),
            });
        }
        Ok(agent)
    }

    fn dispatch_home_script_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent = self.validate_home_extension_invocation(context, tool)?;
        let grant = agent
            .script_grants()
            .into_iter()
            .find(|grant| grant.name == tool.name)
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "home script proxy",
                message: format!("script `{}` is not granted", tool.name),
            })?;
        let Some(environment_name) = grant.environment.as_deref() else {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": {
                        "kind": "missing_environment",
                        "message": format!("script `{}` grant has no environment", grant.name)
                    }
                }),
            });
        };
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let script_registry = script_registry_for_workspace(session.workspace_id());
        let env_registry = environment_registry_for_workspace(session.workspace_id());
        let Some(env) = env_registry.get(environment_name)? else {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": {
                        "kind": "missing_environment",
                        "message": format!("environment `{environment_name}` is not registered on home")
                    }
                }),
            });
        };
        let result = script_registry.execute(&grant.name, &env, arguments)?;
        let payload = if result.logs.is_empty() || !result.ok {
            result.payload
        } else {
            serde_json::json!({
                "result": result.payload,
                "logs": result.logs,
            })
        };
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: result.ok,
            payload,
        })
    }

    async fn dispatch_home_connector_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent = self.validate_home_extension_invocation(context, tool)?;
        let registry = connector_registry()?;
        for grant in agent.connector_grants() {
            let Some(connector) = registry.get(&grant.name)? else {
                continue;
            };
            for operation in &connector.operations {
                if crate::connector::connector_tool_name(&connector.name, &operation.name)
                    != tool.tool_name
                {
                    continue;
                }
                let max_safety =
                    crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
                let vault_service = self
                    .owned
                    .config_projection
                    .snapshot()
                    .user_config
                    .credential_vault
                    .service;
                let connector_name = connector.name.clone();
                let operation_name = operation.name.clone();
                let credential = grant.credential.clone();
                let adapters = connector_adapter_registry()?;
                let prepared = tokio::task::spawn_blocking(move || {
                    registry.prepare_call(
                        &adapters,
                        &connector_name,
                        &operation_name,
                        credential.as_deref(),
                        max_safety,
                        arguments,
                        vault_service,
                    )
                })
                .await
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "home connector proxy",
                    message: error.to_string(),
                })??;
                let execution = self
                    .owned
                    .connector_adapter_processes
                    .execute(&context.worker_provider_run_id, prepared)
                    .await?;
                return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(execution).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "home connector proxy",
                            message: error.to_string(),
                        }
                    })?,
                });
            }
        }
        Ok(crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "error": format!("home connector tool `{}` is not granted", tool.tool_name)
            }),
        })
    }
}
