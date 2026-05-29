use super::capability_registry::mcp_registry_for_workspace;
use super::*;

impl KernelRuntimeState {
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
        if let Some(cached) = self.begin_home_extension_invocation(&metadata).await? {
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
                self.append_home_extension_audit_event(
                    "home_extension.invoke.completed",
                    &context,
                    &metadata,
                    &tool,
                    Some("cancelled"),
                    Some(&error.to_string()),
                )
                .await?;
                return Err(error);
            }
            self.append_home_extension_audit_event(
                "home_extension.invoke.completed",
                &context,
                &metadata,
                &tool,
                Some("completed"),
                None,
            )
            .await?;
        } else {
            self.forget_home_extension_invocation(&metadata).await;
            if let Err(error) = &result {
                self.append_home_extension_audit_event(
                    "home_extension.invoke.completed",
                    &context,
                    &metadata,
                    &tool,
                    Some("failed"),
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
}
