use super::capability_registry::mcp_registry_for_workspace;
use super::*;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_home_mcp_proxy_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        name: String,
        hinted_tool: crate::extension::RemoteExtensionTool,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let tool =
            match super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
                .authorize_invocation(&context, &hinted_tool)
            {
                Ok(tool) => tool,
                Err(error) => {
                    let _ = self
                        .append_home_extension_denied_event(
                            &context,
                            &metadata,
                            &hinted_tool,
                            &error,
                        )
                        .await;
                    return Err(error);
                }
            };
        if name != tool.tool_name {
            let error = DaemonError::LocalTransport {
                operation: "home MCP proxy",
                message: format!(
                    "home MCP proxy name `{name}` does not match authorized home-proxy tool `{}`",
                    tool.tool_name
                ),
            };
            let _ = self
                .append_home_extension_denied_event(&context, &metadata, &tool, &error)
                .await;
            return Err(error);
        }
        if let Some(cached) = self
            .begin_audited_home_extension_invocation(&context, &metadata, &tool)
            .await?
        {
            return Ok(cached);
        }
        self.append_home_extension_audit_event(
            "home_extension.invoke.accepted",
            &context,
            &metadata,
            &tool,
            None,
            None,
        )
        .await?;
        let result = super::home_extension_execution_policy::with_home_extension_timeout(
            &tool,
            "home MCP proxy",
            self.dispatch_home_mcp_proxy_tool(&context, &name, payload),
        )
        .await
        .and_then(super::home_extension_execution_policy::enforce_home_extension_json_result_limit);
        if let Ok(value) = &result {
            let completed = self
                .complete_home_extension_invocation(&metadata, value.clone())
                .await;
            if !completed {
                let error = super::home_extension_execution_policy::home_extension_cancelled_error(
                    &metadata,
                );
                self.append_home_mcp_proxy_result_audit_event(
                    "home_extension.invoke.completed",
                    &context,
                    &metadata,
                    &tool,
                    Some("cancelled"),
                    serde_json::to_vec(value).ok().map(|bytes| bytes.len()),
                    Some(&error.to_string()),
                )
                .await?;
                return Err(error);
            }
            self.append_home_mcp_proxy_result_audit_event(
                "home_extension.invoke.completed",
                &context,
                &metadata,
                &tool,
                Some("completed"),
                serde_json::to_vec(value).ok().map(|bytes| bytes.len()),
                None,
            )
            .await?;
        } else {
            self.forget_home_extension_invocation(&metadata).await;
            if let Err(error) = &result {
                self.append_home_mcp_proxy_result_audit_event(
                    "home_extension.invoke.completed",
                    &context,
                    &metadata,
                    &tool,
                    Some("failed"),
                    None,
                    Some(&error.to_string()),
                )
                .await?;
            }
        }
        result
    }

    pub(in crate::runtime::state::tool_dispatch) async fn dispatch_home_mcp_proxy_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        let session = self
            .owned
            .session_store
            .get_session(&context.home_session_id)?;
        let registry = mcp_registry_for_workspace(session.workspace_id());
        let Some(config) = registry.get(name)? else {
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

    async fn append_home_mcp_proxy_result_audit_event(
        &self,
        kind: &'static str,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: &crate::extension::RemoteExtensionInvocationMetadata,
        tool: &crate::extension::RemoteExtensionTool,
        status: Option<&str>,
        result_bytes: Option<usize>,
        error: Option<&str>,
    ) -> Result<(), DaemonError> {
        let mut payload = self.home_extension_invocation_audit_payload(context, metadata);
        payload.insert("executor".to_string(), serde_json::json!("mcp"));
        payload.insert("invocation".to_string(), serde_json::json!(metadata));
        payload.insert(
            "tool".to_string(),
            serde_json::json!({
                "kind": tool.kind.as_str(),
                "name": tool.name,
                "tool_name": tool.tool_name,
                "safety": tool.safety,
                "timeout_sec": tool.timeout_sec,
                "version_hash": tool.version_hash,
            }),
        );
        payload.insert("status".to_string(), serde_json::json!(status));
        payload.insert("result_bytes".to_string(), serde_json::json!(result_bytes));
        payload.insert("error".to_string(), serde_json::json!(error));
        self.owned.durable_state_store.append_event(
            kind,
            Some(context.home_agent_id.clone()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }
}
