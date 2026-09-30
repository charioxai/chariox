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

    /// A listing served without the agent's Apps (App admission was busy):
    /// once a slot frees, the provider re-reads its tools through the usual
    /// refresh. It waits for the slot rather than retrying on a timer, keeps
    /// at most one refresh pending per agent, and skips the refresh when no
    /// bound App can be listed (stopped, failed or uninstalled).
    pub(super) fn refresh_app_catalog_later(&self, auth_token: &str) {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [run] = runs.as_slice() else {
            return;
        };
        let Ok(agent) = self.app_agent_for_provider_run(run) else {
            return;
        };
        let control = self.app_control().clone();
        // Listed without any App: one that cannot start yet is refreshed
        // when it starts.
        control.note_listing(&agent, &BTreeSet::new());
        if !control.begin_catalog_refresh(agent.id()) {
            return;
        }
        let state = self.clone();
        let (session, agent) = (agent.session_id().to_owned(), agent.id().to_owned());
        tokio::spawn(async move {
            let listable = match control.admit().await {
                Some(permit) => {
                    let (control, state, agent) = (control.clone(), state.clone(), agent.clone());
                    tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        state
                            .owned
                            .agent_store
                            .get_agent(&agent)
                            .is_ok_and(|agent| {
                                control.seed_bound_dormant(&agent);
                                control.has_active_apps_for_agent(&agent)
                            })
                    })
                    .await
                    .unwrap_or(false)
                }
                None => false,
            };
            // A listing saturated again during the refresh schedules another.
            control.end_catalog_refresh(&agent);
            if listable {
                let _ = state
                    .refresh_agent_runtime_tool_catalog(&session, &agent)
                    .await;
            }
        });
    }

    /// Whether the run's agent has a running or dormant bound App.
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

    /// A cheap snapshot only decides whether current App trust needs a bounded
    /// SQLite read. Publication and invocation still check the full catalog.
    pub(super) fn has_app_grants_for_auth_token(&self, auth_token: &str) -> bool {
        let runs = self
            .owned
            .provider_store
            .get_runs_by_runtime_mcp_auth_token(auth_token);
        let [run] = runs.as_slice() else {
            return false;
        };
        self.app_agent_for_provider_run(run).is_ok_and(|agent| {
            agent
                .extension_grants()
                .iter()
                .any(|grant| grant.kind == ExtensionKind::App)
        })
    }

    /// The App tools the agent's provider lists.
    fn app_tools_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
        occupied: &[RuntimeToolSpec],
        permit: Option<&tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<Vec<RemoteExtensionTool>, DaemonError> {
        let occupied = occupied_names(occupied);
        match permit {
            Some(permit) => self
                .app_control()
                .app_extension_listing_admitted(agent, &occupied, permit),
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
        let permit = self
            .app_control()
            .try_admit()
            .map_err(|_| admission_busy())?;
        let state = self.clone();
        let run = provider_run.clone();
        let auth_token = auth_token.to_owned();
        let tool_name = tool_name.to_owned();
        let (agent, tool) = tokio::task::spawn_blocking(move || {
            let agent = state.app_agent_for_provider_run(&run)?;
            let base = state.runtime_tool_specs_without_apps_for_auth_token(&auth_token);
            let tool = state
                .app_control()
                .app_extension_tools_for_agent_admitted(&agent, &occupied_names(&base), &permit)
                .map_err(app_error)?
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
        let permit = self
            .app_control()
            .try_admit()
            .map_err(|_| admission_busy())?;
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

fn occupied_names(specs: &[RuntimeToolSpec]) -> BTreeSet<String> {
    specs.iter().map(|tool| tool.name.clone()).collect()
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
/// Full admission is momentary contention, not an unavailable App.
fn admission_busy() -> DaemonError {
    coded(crate::runtime::app_call_errors::admission_busy())
}
/// A failed App tool call as `CODE: message`, the code a view would see.
fn coded((code, message): (String, String)) -> DaemonError {
    app_error(format!("{code}: {message}"))
}
fn tool_call_error(error: crate::durable_state::app_tools::AppToolsError) -> DaemonError {
    coded(crate::runtime::app_call_errors::tool_call_error(&error))
}
fn worker_call_error(error: crate::runtime::app_worker::AppWorkerError) -> DaemonError {
    coded(crate::runtime::app_call_errors::worker_call_error(&error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::durable_state::app_tools::AppToolsError;
    use chariox_app_runtime::app_catalog::CatalogError;

    #[test]
    fn an_agent_sees_the_view_code_before_the_message() {
        let message = |error: DaemonError| match error {
            DaemonError::LocalTransport { message, .. } => message,
            other => panic!("unexpected error {other:?}"),
        };
        assert_eq!(
            message(tool_call_error(AppToolsError::Catalog(
                CatalogError::Output
            ))),
            "INVALID_OUTPUT: The App's answer does not match the tool's declared output",
        );
        assert_eq!(
            message(worker_call_error(
                crate::runtime::app_worker::AppWorkerError::Deadline
            )),
            "DEADLINE_EXCEEDED: The App did not answer in time",
        );
    }
}
