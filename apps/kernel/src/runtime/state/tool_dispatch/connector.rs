use super::capability_registry::{connector_adapter_registry, connector_registry};
use super::*;

impl KernelRuntimeState {
    pub(super) fn connector_runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
    ) -> Vec<crate::transport::runtime_tools::RuntimeToolSpec> {
        let provider_runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let Some(provider_run) = provider_runs.first() else {
            return Vec::new();
        };
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Vec::new();
        };
        let Ok(agent) = self.owned.agent_store.get_agent(agent_id) else {
            return Vec::new();
        };
        let Ok(registry) = connector_registry() else {
            return Vec::new();
        };
        agent
            .connector_grants()
            .into_iter()
            .filter_map(|grant| {
                registry
                    .get(&grant.name)
                    .ok()
                    .flatten()
                    .map(|connector| (grant, connector))
            })
            .flat_map(|(grant, connector)| {
                let max_safety =
                    crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())
                        .unwrap_or(crate::connector::ConnectorSafety::Read);
                let connector_name = connector.name.clone();
                connector
                    .operations
                    .into_iter()
                    .filter(move |operation| operation.safety <= max_safety)
                    .map(
                        move |operation| crate::transport::runtime_tools::RuntimeToolSpec {
                            name: crate::connector::connector_tool_name(
                                &connector_name,
                                &operation.name,
                            ),
                            description: operation.description,
                            input_schema: operation.input_schema,
                        },
                    )
            })
            .collect()
    }

    pub(super) async fn try_dispatch_connector_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(None);
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let registry = connector_registry()?;
        for grant in agent.connector_grants() {
            let Some(connector) = registry.get(&grant.name)? else {
                continue;
            };
            for operation in &connector.operations {
                if crate::connector::connector_tool_name(&connector.name, &operation.name)
                    != tool_name
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
                    operation: "runtime_tool_connector",
                    message: error.to_string(),
                })??;
                let execution = self
                    .owned
                    .connector_adapter_processes
                    .execute(provider_run.id(), prepared)
                    .await?;
                return Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
                    ok: true,
                    payload: serde_json::to_value(execution).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_connector",
                            message: error.to_string(),
                        }
                    })?,
                }));
            }
        }
        Ok(None)
    }
}
