use super::capability_registry::mcp_registry_for_workspace;
use super::*;

impl KernelRuntimeState {
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
