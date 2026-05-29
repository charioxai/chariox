use arroba_relay::protocol::ClientTarget;

use super::capability_registry::{
    connector_adapter_registry, connector_registry, environment_registry_for_workspace,
    mcp_registry_for_workspace, script_registry_for_workspace,
};
use super::*;

const HOME_EXTENSION_MAX_RESULT_BYTES: usize = 1024 * 1024;

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

    pub(crate) async fn dispatch_forwarded_home_extension_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        hinted_tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let tool = self.validate_home_extension_invocation(&context, &hinted_tool)?;
        if let Some(cached) = self.begin_home_extension_invocation(&metadata).await? {
            return serde_json::from_value(cached).map_err(|error| DaemonError::LocalTransport {
                operation: "home extension invocation replay",
                message: error.to_string(),
            });
        }
        self.append_home_extension_audit_event(
            "home_extension.invoke.accepted",
            &context,
            &metadata,
            &tool,
            None,
        )
        .await?;
        let result = match tool.kind.clone() {
            crate::extension::ExtensionKind::Script => {
                self.dispatch_home_script_tool(&context, &tool, arguments)
            }
            crate::extension::ExtensionKind::Connector => self
                .dispatch_home_connector_tool(&context, &tool, arguments)
                .await
                .and_then(enforce_home_extension_runtime_result_limit),
            _ => Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "home extension runtime tool invocation only supports scripts and connectors"
                }),
            }),
        };
        if let Ok(result) = &result {
            self.complete_home_extension_invocation(
                &metadata,
                serde_json::to_value(result).map_err(|error| DaemonError::LocalTransport {
                    operation: "home extension invocation replay",
                    message: error.to_string(),
                })?,
            )
            .await;
        } else {
            self.forget_home_extension_invocation(&metadata).await;
        }
        match tool.kind.clone() {
            crate::extension::ExtensionKind::Script => {
                self.audit_home_runtime_tool_result("script", &context, &metadata, &tool, &result)
                    .await?;
                result
            }
            crate::extension::ExtensionKind::Connector => {
                self.audit_home_runtime_tool_result(
                    "connector",
                    &context,
                    &metadata,
                    &tool,
                    &result,
                )
                .await?;
                result
            }
            _ => result,
        }
    }

    pub(crate) async fn dispatch_forwarded_home_mcp_proxy_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        name: String,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let hinted_tool = crate::extension::RemoteExtensionTool {
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
        let tool = self.validate_home_extension_invocation(&context, &hinted_tool)?;
        if let Some(cached) = self.begin_home_extension_invocation(&metadata).await? {
            return Ok(cached);
        }
        self.append_home_extension_audit_event(
            "home_extension.invoke.accepted",
            &context,
            &metadata,
            &tool,
            None,
        )
        .await?;
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
        let result = tokio::task::spawn_blocking(move || {
            crate::provider::dispatch_provider_mcp_proxy_request(&config, payload)
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "home MCP proxy",
            message: error.to_string(),
        })?
        .and_then(enforce_home_extension_json_result_limit);
        if let Ok(value) = &result {
            self.complete_home_extension_invocation(&metadata, value.clone())
                .await;
            self.append_home_extension_audit_event(
                "home_extension.invoke.completed",
                &context,
                &metadata,
                &tool,
                Some("completed"),
            )
            .await?;
        } else {
            self.forget_home_extension_invocation(&metadata).await;
        }
        result
    }

    fn validate_home_extension_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        hinted_tool: &crate::extension::RemoteExtensionTool,
    ) -> Result<crate::extension::RemoteExtensionTool, DaemonError> {
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
        if context.worker_kernel_id.as_deref() != Some(remote_execution.worker_kernel_id.as_str()) {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "worker kernel does not match home agent binding".to_string(),
            });
        }
        if context.worker_machine_id.as_deref() != Some(remote_execution.worker_machine_id.as_str())
        {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "worker machine does not match home agent binding".to_string(),
            });
        }
        let manifest = self.remote_extension_manifest_for_agent(&agent)?;
        let Some(current_tool) = manifest.home_proxy_tool(&hinted_tool.tool_name).cloned() else {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: format!(
                    "home-proxy extension tool `{}` is no longer granted",
                    hinted_tool.tool_name
                ),
            });
        };
        if current_tool.kind != hinted_tool.kind || current_tool.name != hinted_tool.name {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: format!(
                    "home-proxy tool identity mismatch for `{}`",
                    hinted_tool.tool_name
                ),
            });
        }
        if current_tool.authority != crate::extension::ExtensionAuthority::Home
            || current_tool.definition_origin != crate::extension::ExtensionDefinitionOrigin::Home
            || current_tool.execution_location != crate::extension::ExtensionExecutionLocation::Home
        {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "home-proxy tool placement is invalid".to_string(),
            });
        }
        Ok(current_tool)
    }

    fn dispatch_home_script_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent = self.validate_home_extension_context_agent(context, tool)?;
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
        enforce_home_extension_runtime_result_limit(
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: result.ok,
                payload,
            },
        )
    }

    async fn dispatch_home_connector_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent = self.validate_home_extension_context_agent(context, tool)?;
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

    fn validate_home_extension_context_agent(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self.owned.agent_store.get_agent(&context.home_agent_id)?;
        if agent.session_id() != context.home_session_id {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "agent does not belong to invocation session".to_string(),
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

    async fn append_home_extension_audit_event(
        &self,
        kind: &'static str,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        tool: &crate::extension::RemoteExtensionTool,
        status: Option<&str>,
    ) -> Result<(), DaemonError> {
        self.owned.durable_state_store.append_event(
            kind,
            Some(context.home_agent_id.clone()),
            serde_json::json!({
                "home_session_id": context.home_session_id,
                "home_agent_id": context.home_agent_id,
                "leased_agent_id": context.leased_agent_id,
                "worker_provider_run_id": context.worker_provider_run_id,
                "invocation": metadata,
                "tool": {
                    "kind": tool.kind.as_str(),
                    "name": tool.name,
                    "tool_name": tool.tool_name,
                    "safety": tool.safety,
                    "timeout_sec": tool.timeout_sec,
                    "version_hash": tool.version_hash,
                },
                "status": status,
            }),
        )?;
        Ok(())
    }

    async fn begin_home_extension_invocation(
        &self,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) -> Result<Option<serde_json::Value>, DaemonError> {
        let key = metadata
            .idempotency_key
            .as_deref()
            .unwrap_or(metadata.invocation_id.as_str())
            .to_string();
        let mut invocations = self.owned.remote_extension_invocations.lock().await;
        if let Some(existing) = invocations.get(&key) {
            if let Some(cached) = existing {
                if metadata.idempotency_key.is_some() {
                    return Ok(Some(cached.clone()));
                }
                return Err(DaemonError::LocalTransport {
                    operation: "home extension invocation replay",
                    message: format!(
                        "duplicate non-idempotent home extension invocation `{}`",
                        metadata.invocation_id
                    ),
                });
            }
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation replay",
                message: format!(
                    "home extension invocation `{}` is already in progress",
                    metadata.invocation_id
                ),
            });
        }
        invocations.insert(key, None);
        Ok(None)
    }

    async fn complete_home_extension_invocation(
        &self,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        result: serde_json::Value,
    ) {
        let key = metadata
            .idempotency_key
            .as_deref()
            .unwrap_or(metadata.invocation_id.as_str())
            .to_string();
        self.owned
            .remote_extension_invocations
            .lock()
            .await
            .insert(key, Some(result));
    }

    async fn forget_home_extension_invocation(
        &self,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
    ) {
        let key = metadata
            .idempotency_key
            .as_deref()
            .unwrap_or(metadata.invocation_id.as_str())
            .to_string();
        self.owned
            .remote_extension_invocations
            .lock()
            .await
            .remove(&key);
    }

    async fn audit_home_runtime_tool_result(
        &self,
        executor: &'static str,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        tool: &crate::extension::RemoteExtensionTool,
        result: &Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError>,
    ) -> Result<(), DaemonError> {
        let (status, ok, result_bytes, error) = match result {
            Ok(result) => (
                "completed",
                Some(result.ok),
                serde_json::to_vec(&result.payload)
                    .ok()
                    .map(|bytes| bytes.len()),
                None,
            ),
            Err(error) => ("failed", None, None, Some(error.to_string())),
        };
        self.owned.durable_state_store.append_event(
            "home_extension.invoke.completed",
            Some(context.home_agent_id.clone()),
            serde_json::json!({
                "executor": executor,
                "home_session_id": context.home_session_id,
                "home_agent_id": context.home_agent_id,
                "leased_agent_id": context.leased_agent_id,
                "worker_provider_run_id": context.worker_provider_run_id,
                "invocation": metadata,
                "tool": {
                    "kind": tool.kind.as_str(),
                    "name": tool.name,
                    "tool_name": tool.tool_name,
                    "safety": tool.safety,
                },
                "status": status,
                "ok": ok,
                "result_bytes": result_bytes,
                "error": error,
            }),
        )?;
        Ok(())
    }
}

fn enforce_home_extension_runtime_result_limit(
    result: crate::transport::runtime_tools::RuntimeToolResult,
) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
    enforce_home_extension_value_size(&result.payload)?;
    Ok(result)
}

fn enforce_home_extension_json_result_limit(
    result: serde_json::Value,
) -> Result<serde_json::Value, DaemonError> {
    enforce_home_extension_value_size(&result)?;
    Ok(result)
}

fn enforce_home_extension_value_size(value: &serde_json::Value) -> Result<(), DaemonError> {
    let size = serde_json::to_vec(value)
        .map_err(|error| DaemonError::LocalTransport {
            operation: "home extension result limit",
            message: error.to_string(),
        })?
        .len();
    if size > HOME_EXTENSION_MAX_RESULT_BYTES {
        return Err(DaemonError::LocalTransport {
            operation: "home extension result limit",
            message: format!(
                "home extension result exceeded {} bytes ({size} bytes)",
                HOME_EXTENSION_MAX_RESULT_BYTES
            ),
        });
    }
    Ok(())
}
