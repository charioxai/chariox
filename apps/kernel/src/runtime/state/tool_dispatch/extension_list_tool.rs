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
            Option<crate::skill::ArrobaSkillPackage>,
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
        if !matches!(kind, "all" | "mcp" | "skill" | "script" | "connector") {
            return Ok((
                crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "error": "kind must be one of: all, mcp, skill, script, connector"
                    }),
                },
                None,
            ));
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
                        "effective_when_requested": "next_provider_launch"
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
                        "effective_when_requested": "now"
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
                        "effective_when_requested": "current_or_next_turn"
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
                        "effective_when_requested": "current_or_next_turn"
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
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
                        "connectors": connectors
                    }
                }),
            },
            None,
        ))
    }
}
