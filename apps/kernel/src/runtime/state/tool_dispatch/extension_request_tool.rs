use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

use super::capability_registry::{
    connector_registry, environment_registry_for_workspace, mcp_registry_for_workspace,
    script_registry_for_workspace, skill_registry_for_workspace,
};

impl KernelRuntimeState {
    pub(super) async fn handle_request_extension_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        session_id: &str,
        arguments: serde_json::Value,
        include_skill_package: bool,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        let args = serde_json::from_value::<crate::transport::runtime_tools::RequestExtensionArgs>(
            arguments,
        )
        .map_err(|error| DaemonError::LocalTransport {
            operation: "runtime_tool_request_extension",
            message: format!("invalid tool arguments: {error}"),
        })?;
        if agent.remote_execution().is_some() && agent.owner_user_id() != session.owner_user_id() {
            return Ok((
                crate::transport::runtime_tools::RuntimeToolResult {
                    ok: false,
                    payload: serde_json::json!({
                        "error": "home-owned extensions for collaborator remote agents must be granted by the home session owner",
                        "agent_ref": agent.agent_ref(),
                        "kind": args.kind,
                        "name": args.name,
                        "authority": "home",
                        "owner_user_id": session.owner_user_id(),
                    }),
                },
                None,
            ));
        }

        let mut skill_payload = serde_json::Value::Null;
        let mut skill_package = None;
        let mcp_registry = mcp_registry_for_workspace(session.workspace_id());
        let skill_registry = skill_registry_for_workspace(session.workspace_id());
        let (agent, effective_when, requires_provider_restart) = match args.kind.as_str() {
            "mcp" => {
                if mcp_registry.get(&args.name)?.is_none() {
                    return Ok((
                        crate::transport::runtime_tools::RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("MCP `{}` is not installed", args.name),
                                "kind": "mcp",
                                "name": args.name,
                            }),
                        },
                        None,
                    ));
                }
                let granted_agent = self
                    .grant_agent_mcp(agent.id(), args.name.clone(), agent.owner_user_id())
                    .await?;
                let (source_attachment_id, previous_prompt) = self
                    .owned
                    .session_store
                    .get_session(session_id)
                    .ok()
                    .and_then(|session| {
                        self.owned
                            .prompt_state_owner
                            .active_prompt_for_agent_or_restore(&session, granted_agent.id())
                            .map(|prompt| {
                                (
                                    prompt.source_attachment_id().to_string(),
                                    prompt.prompt().to_string(),
                                )
                            })
                    })
                    .unwrap_or_else(|| ("arroba-runtime".to_string(), String::new()));
                self.remember_pending_mcp_continuation(
                    session_id,
                    granted_agent.id(),
                    &source_attachment_id,
                    &args.name,
                    &previous_prompt,
                );
                (granted_agent, "after_provider_reload", true)
            }
            "skill" => {
                let Some(skill) = skill_registry.get(&args.name)? else {
                    return Ok((
                        crate::transport::runtime_tools::RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("skill `{}` is not installed", args.name),
                                "kind": "skill",
                                "name": args.name,
                            }),
                        },
                        None,
                    ));
                };
                if include_skill_package {
                    skill_package = skill_registry.package(&args.name)?;
                }
                if args.return_body.unwrap_or(true) {
                    let body = std::fs::read_to_string(&skill.path).map_err(|error| {
                        DaemonError::LocalTransport {
                            operation: "runtime_tool_request_extension",
                            message: format!("failed to read skill `{}` body: {error}", skill.name),
                        }
                    })?;
                    skill_payload = serde_json::json!({
                        "name": skill.name,
                        "description": skill.description,
                        "short_description": skill.short_description,
                        "path": skill.path,
                        "body": body
                    });
                }
                let granted_agent = self
                    .grant_agent_skill(agent.id(), args.name.clone(), agent.owner_user_id())
                    .await?;
                (granted_agent, "now", false)
            }
            "script" => {
                let environment =
                    args.environment
                        .clone()
                        .ok_or_else(|| DaemonError::LocalTransport {
                            operation: "runtime_tool_request_extension",
                            message: "script extension requests require environment".to_string(),
                        })?;
                let script_registry = script_registry_for_workspace(session.workspace_id());
                if script_registry.get(&args.name)?.is_none() {
                    return Ok((
                        crate::transport::runtime_tools::RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("script `{}` is not registered", args.name),
                                "kind": "script",
                                "name": args.name,
                            }),
                        },
                        None,
                    ));
                }
                let environment_registry =
                    environment_registry_for_workspace(session.workspace_id());
                if environment_registry.get(&environment)?.is_none() {
                    return Ok((
                        crate::transport::runtime_tools::RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("environment `{environment}` is not registered"),
                                "kind": "script",
                                "name": args.name,
                                "environment": environment,
                            }),
                        },
                        None,
                    ));
                }
                let granted_agent = self
                    .grant_agent_extension(
                        agent.id(),
                        crate::extension::ExtensionGrant::script(args.name.clone(), environment),
                        agent.owner_user_id(),
                    )
                    .await?;
                (granted_agent, "now", false)
            }
            "connector" => {
                let connector_registry = connector_registry()?;
                if connector_registry.get(&args.name)?.is_none() {
                    return Ok((
                        crate::transport::runtime_tools::RuntimeToolResult {
                            ok: false,
                            payload: serde_json::json!({
                                "error": format!("connector `{}` is not registered", args.name),
                                "kind": "connector",
                                "name": args.name,
                            }),
                        },
                        None,
                    ));
                }
                let max_safety = crate::connector::ConnectorSafety::parse(args.allow.as_deref())?;
                if let Some(credential) = args.credential.as_deref() {
                    crate::runtime::capability_registry::ensure_credential_exists(credential)?;
                }
                let granted_agent = self
                    .grant_agent_extension(
                        agent.id(),
                        crate::extension::ExtensionGrant::connector(
                            args.name.clone(),
                            args.credential.clone(),
                            max_safety.as_str(),
                        ),
                        agent.owner_user_id(),
                    )
                    .await?;
                (granted_agent, "now", false)
            }
            _ => {
                return Ok((
                    crate::transport::runtime_tools::RuntimeToolResult {
                        ok: false,
                        payload: serde_json::json!({
                            "error": "kind must be one of: mcp, skill, script, connector"
                        }),
                    },
                    None,
                ));
            }
        };

        let mut payload = serde_json::json!({
            "granted": true,
            "kind": args.kind,
            "name": args.name,
            "agent_ref": agent.agent_ref(),
            "effective": effective_when,
            "requires_provider_restart": requires_provider_restart,
            "note": match effective_when {
                "after_provider_reload" => "Arroba will reload this provider conversation after the current turn and send an automatic continuation prompt once the MCP is available.",
                "next_provider_launch" => "MCP grants are rendered into provider-native MCP config when the provider run launches; restart/relaunch the agent provider run before using this MCP.",
                "now" => "The extension grant is persisted and available immediately in this turn.",
                _ => "The extension grant is persisted."
            }
        });
        if !skill_payload.is_null() {
            payload["skill"] = skill_payload;
        }
        Ok((
            crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload },
            skill_package,
        ))
    }
}
