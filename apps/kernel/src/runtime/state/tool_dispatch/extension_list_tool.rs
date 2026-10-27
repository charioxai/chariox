use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

use super::capability_registry::{
    connector_registry, mcp_registry_for_workspace, script_registry_for_workspace,
    skill_registry_for_workspace,
};

impl KernelRuntimeState {
    pub(super) fn handle_list_extensions_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::CharioxSkillPackage>,
        ),
        DaemonError,
    > {
        let args = serde_json::from_value::<crate::transport::runtime_tools::ListExtensionsArgs>(
            arguments,
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_list_extensions",
            message: format!("invalid tool arguments: {error}"),
        })?;
        let kind = args.kind.as_deref().unwrap_or("all");
        let remote_home_proxy = agent.remote_execution().is_some();
        let active_execution_location = if remote_home_proxy { "home" } else { "worker" };
        let active_definition_origin = if remote_home_proxy { "home" } else { "worker" };
        if !matches!(
            kind,
            "all" | "mcp" | "skill" | "script" | "connector" | "app"
        ) {
            return Ok((
                crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "error": "kind must be one of: all, mcp, skill, script, connector, app"
                    }),
                },
                None,
            ));
        }
        if args.apps_cursor.is_some() && !matches!(kind, "all" | "app") {
            return Err(DaemonError::LocalTransport {
                operation: "runtime_tool_list_extensions",
                message: "apps_cursor requires kind app or all".into(),
            });
        }

        let mcp_registry = mcp_registry_for_workspace(session.workspace_id());
        let mcps = if matches!(kind, "all" | "mcp") {
            mcp_registry
                .list()?
                .into_iter()
                .map(|mcp| {
                    let granted =
                        agent.has_extension_grant(crate::extension::ExtensionKind::Mcp, &mcp.name);
                    serde_json::json!({
                        "kind": "mcp",
                        "name": mcp.name,
                        "enabled": mcp.enabled,
                        "required": mcp.required,
                        "granted": granted,
                        "authority": if remote_home_proxy { "home" } else { "worker" },
                        "definition_origin": active_definition_origin,
                        "execution_location": active_execution_location,
                        "effective_when_requested": "after_provider_reload",
                        "ready_state": if granted { "granted" } else { "available" }
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let skill_registry = skill_registry_for_workspace(session.workspace_id());
        let skills = if matches!(kind, "all" | "skill") {
            skill_registry
                .list()?
                .into_iter()
                .map(|skill| {
                    let granted = agent
                        .has_extension_grant(crate::extension::ExtensionKind::Skill, &skill.name);
                    serde_json::json!({
                        "kind": "skill",
                        "name": skill.name,
                        "description": skill.description,
                        "short_description": skill.short_description,
                        "granted": granted,
                        "authority": if remote_home_proxy { "home" } else { "worker" },
                        "definition_origin": if remote_home_proxy { "projected_snapshot" } else { "worker" },
                        "execution_location": "none",
                        "effective_when_requested": "now",
                        "ready_state": if granted { "ready" } else { "available" }
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let script_registry = script_registry_for_workspace(session.workspace_id());
        let scripts = if matches!(kind, "all" | "script") {
            script_registry
                .list()?
                .into_iter()
                .map(|script| {
                    let granted = agent
                        .has_extension_grant(crate::extension::ExtensionKind::Script, &script.name);
                    serde_json::json!({
                        "kind": "script",
                        "name": script.name,
                        "description": script.description,
                        "runtime": script.runtime,
                        "definition_hash": script.definition_hash,
                        "granted": granted,
                        "authority": if remote_home_proxy { "home" } else { "worker" },
                        "definition_origin": active_definition_origin,
                        "execution_location": active_execution_location,
                        "effective_when_requested": self.runtime_catalog_grant_effect(agent, granted).0,
                        "ready_state": if granted { "ready" } else { "available" }
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let connector_registry = connector_registry()?;
        let connectors = if matches!(kind, "all" | "connector") {
            connector_registry
                .list()?
                .into_iter()
                .map(|connector| {
                    let grant = agent
                        .connector_grants()
                        .into_iter()
                        .find(|grant| grant.name == connector.name);
                    let max_safety = grant
                        .as_ref()
                        .and_then(|grant| {
                            crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())
                                .ok()
                        })
                        .unwrap_or(crate::connector::ConnectorSafety::Read);
                    let operations = connector
                        .operations
                        .iter()
                        .filter(|operation| operation.safety <= max_safety)
                        .map(|operation| {
                            serde_json::json!({
                                "name": operation.name,
                                "tool_name": crate::connector::connector_tool_name(&connector.name, &operation.name),
                                "description": operation.description,
                                "safety": operation.safety.as_str()
                            })
                        })
                        .collect::<Vec<_>>();
                    serde_json::json!({
                        "kind": "connector",
                        "name": connector.name,
                        "description": connector.description,
                        "adapter": connector.adapter,
                        "granted": grant.is_some(),
                        "authority": if remote_home_proxy { "home" } else { "worker" },
                        "definition_origin": active_definition_origin,
                        "execution_location": active_execution_location,
                        "max_safety": grant.as_ref().and_then(|grant| grant.max_safety.clone()).unwrap_or_else(|| "read".to_string()),
                        "operations": operations,
                        "effective_when_requested": self.runtime_catalog_grant_effect(agent, grant.is_some()).0,
                        "ready_state": if grant.is_some() { "ready" } else { "available" }
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let (apps, apps_next_cursor) = if matches!(kind, "all" | "app") {
            let page = self
                .owned
                .durable_state_store
                .list_app_installations(agent.owner_user_id(), args.apps_cursor.as_deref(), 100)
                .map_err(|_| DaemonError::LocalTransport {
                    operation: "runtime_tool_list_extensions",
                    message: "App installation inventory is unavailable".into(),
                })?;
            // An App whose worker runs or may start on demand is listed once
            // bound (a call starts it); a user stop or a failed generation is
            // not. The start gate answers that without loading the release.
            let control = self.app_control();
            let store = &self.owned.durable_state_store;
            let owner = agent.owner_user_id();
            let apps = page
                .installations
                .into_iter()
                .map(|installation| {
                    let id = installation.installation_id.as_str();
                    let granted =
                        agent.has_extension_grant(crate::extension::ExtensionKind::App, id);
                    let active = installation.active.is_some();
                    let listed = control.active_app_lease(owner, id).is_some()
                        || control.is_app_dormant(owner, id)
                        || matches!(
                            store.app_worker_start_gate(owner, id),
                            Ok(crate::durable_state::app_worker_lifecycle::StartGate::Allowed)
                        );
                    let (ready_state, effective) = app_readiness(active, granted, listed);
                    serde_json::json!({
                        "kind": "app", "name": id,
                        "app_id": installation.app_id,
                        "granted": granted,
                        "active_release": active,
                        "tools_available": granted && listed,
                        "ready_state": ready_state,
                        "effective_when_requested": effective
                    })
                })
                .collect::<Vec<_>>();
            (apps, page.next_cursor)
        } else {
            (Vec::new(), None)
        };

        Ok((
            crate::transport::runtime_tools::RuntimeToolResult {
                ok: true,
                payload: serde_json::json!({
                    "agent_ref": agent.agent_ref(),
                    "extensions": {
                        "mcps": mcps,
                        "skills": skills,
                        "scripts": scripts,
                        "connectors": connectors,
                        "apps": apps,
                        "apps_next_cursor": apps_next_cursor
                    }
                }),
            },
            None,
        ))
    }
}

/// An App's readiness for this agent, and when a request for it takes effect
/// (as `request_extension` answers it). `listed`: it runs or may start on
/// demand, so its tools are in the agent's catalog once bound.
fn app_readiness(active: bool, granted: bool, listed: bool) -> (&'static str, &'static str) {
    match (active, granted, listed) {
        (false, _, _) => ("unavailable", "unavailable"),
        (true, true, true) => ("ready", "now"),
        (true, true, false) => ("stopped", "binding_saved"),
        (true, false, true) => ("available", "after_provider_reload"),
        (true, false, false) => ("stopped", "binding_saved"),
    }
}

#[cfg(test)]
mod app_readiness_tests {
    #[test]
    fn app_readiness_matches_what_a_request_answers() {
        use super::app_readiness;
        assert_eq!(
            app_readiness(false, true, true),
            ("unavailable", "unavailable")
        );
        assert_eq!(app_readiness(true, true, true), ("ready", "now"));
        assert_eq!(
            app_readiness(true, true, false),
            ("stopped", "binding_saved")
        );
        assert_eq!(
            app_readiness(true, false, true),
            ("available", "after_provider_reload")
        );
        // Unbound but stopped by the user or failed: a request only saves it.
        assert_eq!(
            app_readiness(true, false, false),
            ("stopped", "binding_saved")
        );
    }
}
