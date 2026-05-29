use super::capability_registry::{
    environment_registry_for_workspace, script_registry_for_workspace,
};
use super::home_extension_authorizer::HomeExtensionAuthorizationService;
use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn dispatch_home_script_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent =
            HomeExtensionAuthorizationService::new(self).authorize_granted_agent(context, tool)?;
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
        let script_name = grant.name;
        tokio::task::spawn_blocking(move || {
            let result = script_registry.execute(&script_name, &env, arguments)?;
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
        })
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "home script proxy",
            message: error.to_string(),
        })?
    }
}
