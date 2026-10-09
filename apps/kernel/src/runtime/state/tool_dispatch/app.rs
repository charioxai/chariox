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
        if run.owner_user_id() != agent.owner_user_id()
            || run.state() == crate::provider::ProviderRunState::Ended
            || self
                .owned
                .provider_store
                .get_run_for_agent(agent.session_id(), agent.id())
                .is_none_or(|current| {
                    current.id() != run.id()
                        || current.state() == crate::provider::ProviderRunState::Ended
                })
            || self
                .owned
                .session_store
                .get_session(agent.session_id())?
                .status()
                == crate::session::SessionStatus::Ended
        {
            return Err(unavailable());
        }
        if self.room_agent_tools_enabled()
            && (agent.remote_execution().is_some()
                || self.slice_kernel_id().is_some()
                || self
                    .owned
                    .provider_run_projection
                    .is_leased_provider_run(run.id()))
        {
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
        let app_shaped = has_app_tool_shape(tool_name);
        let state = self.clone();
        let run = provider_run.clone();
        let auth_token = auth_token.to_owned();
        let name = tool_name.to_owned();
        let (agent, tool, published_elsewhere) = tokio::task::spawn_blocking(move || {
            let agent = state.app_agent_for_provider_run(&run)?;
            let base = state.runtime_tool_specs_without_apps_for_auth_token(&auth_token);
            let published_elsewhere = base.iter().any(|tool| tool.name == name);
            let tool = state
                .app_control()
                .app_extension_tools_for_agent_admitted(&agent, &occupied_names(&base), &permit)
                .map_err(app_error)?
                .into_iter()
                .find(|tool| tool.tool_name == name);
            Ok::<_, DaemonError>((agent, tool, published_elsewhere))
        })
        .await
        .map_err(|_| unavailable())??;
        let Some(tool) = tool else {
            // A bound App's tools are withdrawn while an update replaces it.
            if self.app_tool_updating(&agent, tool_name).await {
                return Err(coded(crate::runtime::app_call_errors::updating()));
            }
            // An App tool none of the agent's bound Apps offers: the binding was
            // revoked, or the provider still lists a withdrawn tool (Codex keeps
            // its list for the turn). Refuse it here; the dispatchers after this
            // one would answer with an unrelated workflow-turn error.
            // Scripts and connectors can publish the same name shape. Keep
            // routing those names to their own authenticated dispatchers.
            if app_shaped && !published_elsewhere {
                return Err(unavailable());
            }
            return Ok(None);
        };
        self.invoke_bound_app_tool(&agent, provider_run.id(), &tool, input, None)
            .await
            .map(Some)
    }

    /// Whether `tool_name` names a tool of one of the agent's bound Apps whose
    /// approved update is in progress.
    async fn app_tool_updating(
        &self,
        agent: &crate::agent::AgentInstance,
        tool_name: &str,
    ) -> bool {
        let Some(installation) = agent
            .granted_extension_names(ExtensionKind::App)
            .into_iter()
            .find(|installation| {
                chariox_app_runtime::app_catalog::is_installation_tool_name(tool_name, installation)
            })
        else {
            return false;
        };
        self.app_updating(agent.owner_user_id(), &installation)
            .await
    }

    pub(in crate::runtime::state::tool_dispatch) async fn dispatch_home_app_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        tool: &RemoteExtensionTool,
        input: serde_json::Value,
    ) -> Result<RuntimeToolResult, DaemonError> {
        if self.room_agent_tools_enabled() {
            return Err(unavailable());
        }
        let agent = super::home_extension_authorizer::HomeExtensionAuthorizationService::new(self)
            .authorize_granted_agent(context, tool)?;
        self.invoke_bound_app_tool(
            &agent,
            &context.worker_provider_run_id,
            tool,
            input,
            Some(context.clone()),
        )
        .await
    }

    pub(super) async fn authorize_forwarded_app_tool(
        &self,
        context: &crate::transport::relay_peer::RemoteExtensionInvocationContext,
        hinted: &RemoteExtensionTool,
    ) -> Result<RemoteExtensionTool, DaemonError> {
        if self.room_agent_tools_enabled() {
            return Err(unavailable()); // MP-08: leased agents use Room Browser/Computer only.
        }
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
                .find(|tool| tool.tool_name == hinted.tool_name);
            let Some(current) = current else {
                // A bound App's tools are withdrawn while an update replaces it.
                let updating = agent.has_extension_grant(ExtensionKind::App, &hinted.name)
                    && state
                        .owned
                        .durable_state_store
                        .app_update_underway(agent.owner_user_id(), &hinted.name)
                        .unwrap_or(false);
                return Err(if updating {
                    coded(crate::runtime::app_call_errors::updating())
                } else {
                    unavailable()
                });
            };
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
/// `app_<local>_<32 hex>`, the shape of every App tool name
/// (`chariox_app_runtime::app_catalog`), whichever installation it names.
fn has_app_tool_shape(name: &str) -> bool {
    name.strip_prefix("app_")
        .and_then(|rest| rest.rsplit_once('_'))
        .is_some_and(|(local, digest)| {
            !local.is_empty()
                && digest.len() == 32
                && digest
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
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

    #[test]
    fn only_app_tool_names_are_refused_as_unbound_app_calls() {
        // A live App tool name (drill-actor `record`).
        assert!(has_app_tool_shape(
            "app_record_3fb6e439d5632a7d966fa946a2899d58"
        ));
        assert!(has_app_tool_shape(
            "app_list_todos_0123456789abcdef0123456789abcdef"
        ));
        // Anything else keeps falling through to the other dispatchers.
        assert!(!has_app_tool_shape("app_record"));
        assert!(!has_app_tool_shape("app__0123456789abcdef0123456789abcdef"));
        assert!(!has_app_tool_shape(
            "app_record_0123456789ABCDEF0123456789ABCDEF"
        ));
        assert!(!has_app_tool_shape("app_record_0123456789abcdef"));
        assert!(!has_app_tool_shape("apply_patch"));
        assert!(!has_app_tool_shape("chariox_slice_browser_click"));
    }
}
