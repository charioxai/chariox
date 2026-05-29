use super::*;

pub(in crate::runtime::state::tool_dispatch) struct HomeExtensionAuthorizationService<'a> {
    state: &'a KernelRuntimeState,
}

impl<'a> HomeExtensionAuthorizationService<'a> {
    pub(in crate::runtime::state::tool_dispatch) fn new(state: &'a KernelRuntimeState) -> Self {
        Self { state }
    }

    pub(in crate::runtime::state::tool_dispatch) fn authorize_invocation(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        hinted_tool: &crate::extension::RemoteExtensionTool,
    ) -> Result<crate::extension::RemoteExtensionTool, DaemonError> {
        let agent = self.authorize_invocation_context(context)?;
        let manifest = self.state.remote_extension_manifest_for_agent(&agent)?;
        let Some(current_tool) = manifest.home_proxy_tool(&hinted_tool.tool_name).cloned() else {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: format!(
                    "home-proxy extension tool `{}` is no longer granted",
                    hinted_tool.tool_name
                ),
            });
        };
        validate_projected_tool_matches_current(&current_tool, hinted_tool)?;
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

    pub(in crate::runtime::state::tool_dispatch) fn authorize_invocation_context(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let config = self.state.owned.config_projection.snapshot();
        if config.daemon_id != context.home_kernel_id {
            return Err(DaemonError::LocalTransport {
                operation: "home extension invocation",
                message: "invocation was sent to the wrong home kernel".to_string(),
            });
        }
        let agent = self
            .state
            .owned
            .agent_store
            .get_agent(&context.home_agent_id)?;
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
        Ok(agent)
    }

    pub(in crate::runtime::state::tool_dispatch) fn authorize_granted_agent(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .state
            .owned
            .agent_store
            .get_agent(&context.home_agent_id)?;
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
}

fn validate_projected_tool_matches_current(
    current_tool: &crate::extension::RemoteExtensionTool,
    hinted_tool: &crate::extension::RemoteExtensionTool,
) -> Result<(), DaemonError> {
    if current_tool.kind != hinted_tool.kind || current_tool.name != hinted_tool.name {
        return Err(DaemonError::LocalTransport {
            operation: "home extension invocation",
            message: format!(
                "home-proxy tool identity mismatch for `{}`",
                hinted_tool.tool_name
            ),
        });
    }
    if current_tool.tool_name != hinted_tool.tool_name {
        return Err(DaemonError::LocalTransport {
            operation: "home extension invocation",
            message: format!(
                "home-proxy tool identity mismatch for `{}`",
                hinted_tool.tool_name
            ),
        });
    }
    if current_tool.safety != hinted_tool.safety
        || current_tool.timeout_sec != hinted_tool.timeout_sec
        || current_tool.version_hash != hinted_tool.version_hash
    {
        return Err(DaemonError::LocalTransport {
            operation: "home extension invocation",
            message: format!(
                "home-proxy tool projection for `{}` is stale; retry after manifest sync",
                hinted_tool.tool_name
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(version_hash: Option<&str>) -> crate::extension::RemoteExtensionTool {
        crate::extension::RemoteExtensionTool {
            kind: crate::extension::ExtensionKind::Mcp,
            name: "home_echo_mcp".to_string(),
            tool_name: "home_echo_mcp".to_string(),
            description: "Home MCP".to_string(),
            input_schema: serde_json::json!({}),
            authority: crate::extension::ExtensionAuthority::Home,
            definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
            execution_location: crate::extension::ExtensionExecutionLocation::Home,
            safety: None,
            timeout_sec: Some(10),
            version_hash: version_hash.map(str::to_string),
        }
    }

    #[test]
    fn authorizer_rejects_stale_projected_version_hash() {
        let current = tool(Some("hash-current"));
        let hinted = tool(Some("hash-stale"));
        let error = validate_projected_tool_matches_current(&current, &hinted)
            .expect_err("stale projected version hash should be rejected");
        assert!(
            error
                .to_string()
                .contains("stale; retry after manifest sync"),
            "{error}"
        );
    }

    #[test]
    fn authorizer_rejects_stale_projected_timeout() {
        let current = tool(Some("hash-current"));
        let mut hinted = tool(Some("hash-current"));
        hinted.timeout_sec = Some(5);
        let error = validate_projected_tool_matches_current(&current, &hinted)
            .expect_err("stale projected timeout should be rejected");
        assert!(
            error
                .to_string()
                .contains("stale; retry after manifest sync"),
            "{error}"
        );
    }
}
