use super::home_extension_authorizer::HomeExtensionAuthorizationService;
use super::*;

impl KernelRuntimeState {
    pub(crate) async fn dispatch_forwarded_home_extension_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteExtensionInvocationContext,
        metadata: crate::extension::RemoteExtensionInvocationMetadata,
        hinted_tool: crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let tool = HomeExtensionAuthorizationService::new(self)
            .authorize_invocation(&context, &hinted_tool)?;
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
                .and_then(super::home_extension_execution_policy::enforce_home_extension_runtime_result_limit),
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
        let tool = HomeExtensionAuthorizationService::new(self)
            .authorize_invocation(&context, &hinted_tool)?;
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
        let result = self
            .dispatch_home_mcp_proxy_tool(&context, &name, payload)
            .await
            .and_then(
                super::home_extension_execution_policy::enforce_home_extension_json_result_limit,
            );
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
}
