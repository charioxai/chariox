use super::capability_registry::{connector_adapter_registry, connector_registry};
use super::home_extension_authorizer::HomeExtensionAuthorizationService;
use super::*;

impl KernelRuntimeState {
    pub(in crate::runtime::state::tool_dispatch) async fn dispatch_home_connector_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &crate::extension::RemoteExtensionTool,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let agent =
            HomeExtensionAuthorizationService::new(self).authorize_granted_agent(context, tool)?;
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
