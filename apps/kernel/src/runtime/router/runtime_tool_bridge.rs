use super::CommandRouter;
use crate::error::DaemonError;
use crate::runtime::runtime_mcp_proxy_dispatcher::dispatch_authenticated_mcp_proxy_call as dispatch_runtime_mcp_proxy_call;

impl CommandRouter {
    pub(crate) fn runtime_mcp_bind_address(&self) -> (String, u16) {
        let config = self.config_projection.snapshot();
        (config.runtime_mcp_host, config.runtime_mcp_port)
    }

    pub(crate) async fn dispatch_authenticated_mcp_proxy_call(
        &self,
        auth_token: &str,
        name: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, DaemonError> {
        dispatch_runtime_mcp_proxy_call(&self.provider_run_projection, auth_token, name, payload)
            .await
    }

    pub(crate) async fn dispatch_authenticated_runtime_tool_call(
        &self,
        auth_token: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_authenticated_runtime_tool_call(auth_token, tool_name, arguments)
            .await
    }

    pub(crate) fn runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        self.runtime_state
            .runtime_tool_specs_for_auth_token(auth_token)
    }

    pub(crate) async fn dispatch_forwarded_workflow_runtime_tool_call(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        self.runtime_state
            .dispatch_forwarded_workflow_runtime_tool_call(context, tool_name, arguments)
            .await
    }

    pub(crate) async fn dispatch_forwarded_workflow_provider_failure(
        &self,
        context: crate::execution_lease::RemoteWorkflowTurnContext,
        message: String,
    ) -> Result<(), DaemonError> {
        self.runtime_state
            .dispatch_forwarded_workflow_provider_failure(context, message)
            .await
    }

    pub(crate) async fn dispatch_forwarded_managed_io_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
        artifact_states: Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Vec<crate::transport::relay_peer::RemoteManagedIoArtifactState>,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_managed_io_runtime_tool_call(
                context,
                tool_name,
                arguments,
                artifact_states,
            )
            .await
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        self.runtime_state
            .dispatch_forwarded_capability_runtime_tool_call(context, tool_name, arguments)
            .await
    }
}
