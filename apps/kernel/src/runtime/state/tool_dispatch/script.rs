use super::capability_registry::{
    environment_registry_for_workspace, script_registry_for_workspace,
};
use super::*;

impl KernelRuntimeState {
    pub(super) fn script_runtime_tool_specs_for_auth_token(
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
        let Ok(session) = self
            .owned
            .session_store
            .get_session(provider_run.session_id())
        else {
            return Vec::new();
        };
        let registry = script_registry_for_workspace(session.workspace_id());
        agent
            .script_grants()
            .into_iter()
            .filter_map(|grant| registry.get(&grant.name).ok().flatten())
            .map(|script| crate::transport::runtime_tools::RuntimeToolSpec {
                name: script.name,
                description: script.description,
                input_schema: script.input_schema,
            })
            .collect()
    }

    pub(super) async fn try_dispatch_script_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id() else {
            return Ok(None);
        };
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let Some(grant) = agent
            .script_grants()
            .into_iter()
            .find(|grant| grant.name == tool_name)
        else {
            return Ok(None);
        };
        let Some(environment_name) = grant.environment.as_deref() else {
            return Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": {
                        "kind": "missing_environment",
                        "message": format!("script `{}` grant has no environment", grant.name)
                    }
                }),
            }));
        };
        let session = self
            .owned
            .session_store
            .get_session(provider_run.session_id())?;
        let script_registry = script_registry_for_workspace(session.workspace_id());
        let env_registry = environment_registry_for_workspace(session.workspace_id());
        let Some(env) = env_registry.get(environment_name)? else {
            return Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": {
                        "kind": "missing_environment",
                        "message": format!("environment `{environment_name}` is not registered")
                    }
                }),
            }));
        };
        let result = script_registry.execute(&grant.name, &env, arguments)?;
        let payload = if result.logs.is_empty() || !result.ok {
            result.payload
        } else {
            serde_json::json!({
                "result": result.payload,
                "logs": result.logs,
            })
        };
        Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
            ok: result.ok,
            payload,
        }))
    }
}
