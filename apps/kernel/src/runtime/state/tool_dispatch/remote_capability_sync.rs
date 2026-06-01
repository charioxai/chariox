use arroba_relay::protocol::ClientTarget;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;
use crate::transport::relay_peer::{RelayPeerRequest, RelayPeerResponse};

use super::capability_registry::{
    format_remote_mcp_unavailable, mcp_registry_for_workspace, package_granted_skills,
    required_remote_mcps, skill_registry_for_workspace,
};

impl KernelRuntimeState {
    pub(in crate::runtime::state) async fn ensure_remote_skill_packages_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Vec<crate::transport::relay_peer::RemoteSkillMaterialization>, DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(Vec::new());
        };
        let skill_grants = agent.skill_grants();
        if skill_grants.is_empty() {
            return Ok(Vec::new());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let skill_registry = skill_registry_for_workspace(session.workspace_id());
        let packages = package_granted_skills(&skill_registry, &skill_grants)?;
        if packages.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .with_app_side_effect(|app| {
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::EnsureRemoteSkillPackages {
                            context: crate::transport::relay_peer::RemoteSkillSyncContext {
                                home_kernel_id: app.config().daemon_id.clone(),
                                home_session_id: agent.session_id().to_string(),
                                home_agent_id: agent.id().to_string(),
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                            },
                            packages: packages.clone(),
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::RemoteSkillPackagesEnsured { materialized } => Ok(materialized),
            other => Err(DaemonError::LocalTransport {
                operation: "ensure remote skill packages",
                message: format!("unexpected remote skill sync response: {other:?}"),
            }),
        }
    }

    pub(in crate::runtime::state) fn required_remote_mcps_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<Vec<crate::transport::relay_peer::RequiredRemoteMcp>, DaemonError> {
        if agent.remote_execution().is_some() {
            return Ok(Vec::new());
        }
        let mcp_grants = agent.mcp_grants();
        if mcp_grants.is_empty() {
            return Ok(Vec::new());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let registry = mcp_registry_for_workspace(session.workspace_id());
        required_remote_mcps(&registry, &mcp_grants)
    }

    pub(in crate::runtime::state) fn remote_extension_manifest_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<crate::extension::RemoteExtensionManifest, DaemonError> {
        if agent.remote_execution().is_none() {
            return Ok(crate::extension::RemoteExtensionManifest::default());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let mut tools = Vec::new();

        let mcp_registry = mcp_registry_for_workspace(session.workspace_id());
        for name in agent.mcp_grants() {
            let Some(config) = mcp_registry.get(&name)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote extension manifest",
                    message: format!("MCP `{name}` is granted but is not installed on home"),
                });
            };
            tools.push(crate::extension::RemoteExtensionTool {
                kind: crate::extension::ExtensionKind::Mcp,
                name: name.clone(),
                tool_name: name,
                description: "Home-proxied MCP server".to_string(),
                input_schema: serde_json::json!({}),
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: config.tool_timeout_sec,
                version_hash: Some(config.definition_hash()?),
            });
        }

        let script_registry =
            super::capability_registry::script_registry_for_workspace(session.workspace_id());
        for grant in agent.script_grants() {
            let Some(script) = script_registry.get(&grant.name)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote extension manifest",
                    message: format!(
                        "script `{}` is granted but is not registered on home",
                        grant.name
                    ),
                });
            };
            tools.push(crate::extension::RemoteExtensionTool {
                kind: crate::extension::ExtensionKind::Script,
                name: grant.name,
                tool_name: script.name,
                description: script.description,
                input_schema: script.input_schema,
                authority: crate::extension::ExtensionAuthority::Home,
                definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                execution_location: crate::extension::ExtensionExecutionLocation::Home,
                safety: None,
                timeout_sec: Some(
                    script
                        .timeout_sec
                        .unwrap_or(crate::script::DEFAULT_SCRIPT_EXECUTION_TIMEOUT_SEC),
                ),
                version_hash: Some(script.definition_hash),
            });
        }

        let connector_registry = super::capability_registry::connector_registry()?;
        for grant in agent.connector_grants() {
            let Some(connector) = connector_registry.get(&grant.name)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote extension manifest",
                    message: format!(
                        "connector `{}` is granted but is not registered on home",
                        grant.name
                    ),
                });
            };
            let max_safety = crate::connector::ConnectorSafety::parse(grant.max_safety.as_deref())?;
            let definition_hash = connector.definition_hash()?;
            for operation in connector.operations {
                if operation.safety > max_safety {
                    continue;
                }
                tools.push(crate::extension::RemoteExtensionTool {
                    kind: crate::extension::ExtensionKind::Connector,
                    name: connector.name.clone(),
                    tool_name: crate::connector::connector_tool_name(
                        &connector.name,
                        &operation.name,
                    ),
                    description: operation.description,
                    input_schema: operation.input_schema,
                    authority: crate::extension::ExtensionAuthority::Home,
                    definition_origin: crate::extension::ExtensionDefinitionOrigin::Home,
                    execution_location: crate::extension::ExtensionExecutionLocation::Home,
                    safety: Some(operation.safety.as_str().to_string()),
                    timeout_sec: Some(connector.timeout_ms / 1000),
                    version_hash: Some(definition_hash.clone()),
                });
            }
        }

        Ok(crate::extension::RemoteExtensionManifest { tools })
    }

    pub(in crate::runtime::state) async fn ensure_remote_mcp_availability_for_agent(
        &self,
        agent: &crate::agent::AgentInstance,
    ) -> Result<(), DaemonError> {
        let Some(remote_execution) = agent.remote_execution().cloned() else {
            return Ok(());
        };
        let required_mcps = self.required_remote_mcps_for_agent(agent)?;
        if required_mcps.is_empty() {
            return Ok(());
        }
        let response = self
            .with_app_side_effect(|app| {
                let relay_config = app.relay_config_for_remote_execution(&remote_execution);
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        &relay_config,
                        ClientTarget {
                            daemon_id: Some(remote_execution.worker_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::CheckRemoteMcpAvailability {
                            context: crate::transport::relay_peer::RemoteMcpCheckContext {
                                home_kernel_id: app.config().daemon_id.clone(),
                                home_session_id: agent.session_id().to_string(),
                                home_agent_id: agent.id().to_string(),
                                leased_agent_id: remote_execution.leased_agent_id.clone(),
                            },
                            required_mcps,
                        },
                    ),
                )
            })
            .await?;
        match response {
            RelayPeerResponse::RemoteMcpAvailabilityChecked { results } => {
                let unavailable = results
                    .iter()
                    .filter(|result| {
                        !matches!(
                            result.status,
                            crate::transport::relay_peer::RemoteMcpAvailabilityStatus::Available
                        )
                    })
                    .collect::<Vec<_>>();
                if unavailable.is_empty() {
                    Ok(())
                } else {
                    Err(DaemonError::LocalTransport {
                        operation: "remote mcp availability",
                        message: format_remote_mcp_unavailable(&unavailable),
                    })
                }
            }
            other => Err(DaemonError::LocalTransport {
                operation: "remote mcp availability",
                message: format!("unexpected remote MCP availability response: {other:?}"),
            }),
        }
    }

    pub(in crate::runtime::state) fn apply_remote_materialized_skill_prompt_context(
        &self,
        agent: &crate::agent::AgentInstance,
        prompt: &str,
        materialized: &[crate::transport::relay_peer::RemoteSkillMaterialization],
    ) -> Result<String, DaemonError> {
        let skill_grants = agent.skill_grants();
        if skill_grants.is_empty() {
            return Ok(prompt.to_string());
        }
        let session = self.owned.session_store.get_session(agent.session_id())?;
        let registry = skill_registry_for_workspace(session.workspace_id());
        let materialized_by_name = materialized
            .iter()
            .map(|entry| (entry.name.as_str(), entry))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut lines = vec![
            "Available Arroba skills for this remote agent:".to_string(),
            "These granted skills were synchronized from the home kernel and materialized on this worker before this prompt. Follow them when they match the task; assets, scripts, and references are available under each materialized_root.".to_string(),
        ];
        let mut bodies = Vec::new();
        for grant in skill_grants {
            let Some(skill) = registry.get(&grant)? else {
                return Err(DaemonError::LocalTransport {
                    operation: "remote provider.prompt.skills",
                    message: format!(
                        "agent `{}` has missing skill grant `{grant}`",
                        agent.agent_ref()
                    ),
                });
            };
            let materialized = materialized_by_name
                .get(skill.name.as_str())
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "remote provider.prompt.skills",
                    message: format!("remote skill `{}` was not materialized", skill.name),
                })?;
            let summary = skill
                .short_description
                .as_ref()
                .unwrap_or(&skill.description);
            lines.push(format!(
                "- `{}`: {}; materialized_root: {}; version: {}",
                skill.name, summary, materialized.materialized_root, materialized.version_hash
            ));
            let body = std::fs::read_to_string(&skill.path).map_err(|error| {
                DaemonError::LocalTransport {
                    operation: "remote provider.prompt.skills",
                    message: format!(
                        "failed to read skill `{}` body at `{}`: {error}",
                        skill.name,
                        skill.path.display()
                    ),
                }
            })?;
            bodies.push((skill.name, materialized.materialized_root.clone(), body));
        }
        lines.push(String::new());
        lines.push("Full instructions for synchronized Arroba skills:".to_string());
        for (name, materialized_root, body) in bodies {
            lines.push(format!(
                "<arroba_skill name=\"{name}\" materialized_root=\"{materialized_root}\">"
            ));
            lines.push(body.trim().to_string());
            lines.push("</arroba_skill>".to_string());
        }
        Ok(format!("{}\n\n{}", lines.join("\n"), prompt))
    }
}
