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
