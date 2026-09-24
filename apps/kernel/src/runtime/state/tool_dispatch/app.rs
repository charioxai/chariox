//! App operations join the existing authenticated runtime MCP. A saved binding
//! grants no process, catalog, filesystem path or alternate provider endpoint.

use super::*;
use crate::{
    extension::{ExtensionKind, RemoteExtensionTool},
    transport::runtime_tools::{RuntimeToolResult, RuntimeToolSpec},
};
use std::collections::BTreeSet;

mod invocation;

impl KernelRuntimeState {
    pub(super) fn app_runtime_tool_specs_for_auth_token(
        &self,
        auth_token: &str,
        occupied: &[RuntimeToolSpec],
        permit: Option<&tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<Vec<RuntimeToolSpec>, DaemonError> {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [run] = runs.as_slice() else {
            return Ok(Vec::new());
        };
        let Ok(agent) = self.app_agent_for_provider_run(run) else {
            return Ok(Vec::new());
        };
        Ok(self
            .app_tools_for_agent(&agent, occupied, permit)?
            .into_iter()
            .map(|tool| RuntimeToolSpec {
                name: tool.tool_name,
                description: tool.description,
                input_schema: tool.input_schema,
            })
            .collect())
    }

    /// A cheap snapshot only decides whether current App trust needs a bounded
    /// SQLite read. Publication and invocation still check the full catalog.
    pub(super) fn has_active_apps_for_auth_token(&self, auth_token: &str) -> bool {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [run] = runs.as_slice() else {
            return false;
        };
        self.app_agent_for_provider_run(run)
            .is_ok_and(|agent| self.app_control().has_active_apps_for_agent(&agent))
    }

    fn app_tools_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
        occupied: &[RuntimeToolSpec],
        permit: Option<&tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<Vec<RemoteExtensionTool>, DaemonError> {
        let occupied = occupied
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<BTreeSet<_>>();
        match permit {
            Some(permit) => self
                .app_control()
                .app_extension_tools_for_agent_admitted(agent, &occupied, permit),
            None => self
                .app_control()
                .app_extension_tools_for_agent(agent, &occupied),
        }
        .map_err(app_error)
    }

    fn app_agent_for_provider_run(
        &self,
        run: &crate::provider::RuntimeProviderRun,
    ) -> Result<crate::agent::AgentInstance, DaemonError> {
        let agent = self
            .owned
            .agent_store
            .get_agent(run.agent_instance_id().ok_or_else(unavailable)?)?;
        if run.session_id() != agent.session_id() {
            return Err(unavailable());
        }
        Ok(agent)
    }

    pub(super) async fn try_dispatch_app_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        auth_token: &str,
        tool_name: &str,
        input: serde_json::Value,
    ) -> Result<Option<RuntimeToolResult>, DaemonError> {
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
        let state = self.clone();
        let run = provider_run.clone();
        let auth_token = auth_token.to_owned();
        let tool_name = tool_name.to_owned();
        let (agent, tool) = tokio::task::spawn_blocking(move || {
            let agent = state.app_agent_for_provider_run(&run)?;
            let base = state.runtime_tool_specs_without_apps_for_auth_token(&auth_token);
            let tool = state
                .app_tools_for_agent(&agent, &base, Some(&permit))?
                .into_iter()
                .find(|tool| tool.tool_name == tool_name);
            Ok::<_, DaemonError>((agent, tool))
        })
        .await
        .map_err(|_| unavailable())??;
        let Some(tool) = tool else {
            return Ok(None);
        };
        self.invoke_bound_app_tool(&agent, &tool, input, None)
            .await
            .map(Some)
    }

    pub(in crate::runtime::state::tool_dispatch) async fn dispatch_home_app_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &RemoteExtensionTool,
        input: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        let agent = super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
            .authorize_granted_agent(context, tool)?;
        self.invoke_bound_app_tool(&agent, tool, input, Some(context.clone()))
            .await
    }

    pub(super) async fn authorize_forwarded_app_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        hinted: &RemoteExtensionTool,
    ) -> Result<RemoteExtensionTool, DaemonError> {
        let permit = self.app_control().try_admit().map_err(|_| unavailable())?;
        let state = self.clone();
        let context = context.clone();
        let hinted = hinted.clone();
        tokio::task::spawn_blocking(move || {
            let agent =
                super::home_extension_authorizer::HomeExtensionAuthorizationService::new(&state)
                    .authorize_invocation_context(&context)?;
            let base = state.remote_extension_manifest_without_apps_for_agent(&agent)?;
            let occupied = base
                .tools
                .iter()
                .map(|tool| tool.tool_name.clone())
                .collect();
            let current = state
                .app_control()
                .app_extension_tools_for_agent_admitted(&agent, &occupied, &permit)
                .map_err(app_error)?
                .into_iter()
                .find(|tool| tool.tool_name == hinted.tool_name)
                .ok_or_else(unavailable)?;
            super::home_extension_authorizer::validate_projected_tool_matches_current(
                &current, &hinted,
            )?;
            Ok(current)
        })
        .await
        .map_err(|_| unavailable())?
    }
}

fn unavailable() -> DaemonError {
    DaemonError::LocalTransport {
        operation: "app.tools",
        message: "App operation is not currently available to this agent".into(),
    }
}
fn app_error(error: impl std::fmt::Display) -> DaemonError {
    DaemonError::LocalTransport {
        operation: "app.tools",
        message: error.to_string(),
    }
}
