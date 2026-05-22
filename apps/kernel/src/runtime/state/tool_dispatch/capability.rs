use super::capability_registry::{
    connector_registry, mcp_registry_for_workspace, script_registry_for_workspace,
    skill_registry_for_workspace,
};
use super::*;

impl KernelRuntimeState {
    pub(super) async fn try_dispatch_remote_capability_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let workspace_context = self
            .managed_io_workspace_for_provider_run(provider_run)
            .await?;
        let remote_context = self
            .with_app_side_effect(|app| {
                crate::app::RemoteLeaseRuntime::new(app).leased_managed_io_context_for_provider_run(
                    provider_run.id(),
                    workspace_context.identity.clone(),
                )
            })
            .await;
        let Some(remote_context) = remote_context else {
            return Ok(None);
        };
        if !workspace_context.valid {
            return Ok(Some(managed_io_workspace_identity_rejected(
                &workspace_context,
            )));
        }
        let response = self
            .with_app_side_effect(|app| {
                app.block_on_relay_future(
                    crate::transport::relay_client::send_peer_request_via_temporary_connection(
                        app.config(),
                        ClientTarget {
                            daemon_id: Some(remote_context.home_kernel_id.clone()),
                            daemon_alias: None,
                        },
                        RelayPeerRequest::ForwardCapabilityRuntimeTool {
                            context: remote_context.clone(),
                            tool_name: tool_name.to_string(),
                            arguments: arguments.clone(),
                        },
                    ),
                )
            })
            .await?;
        let (mut result, skill_package) = match response {
            RelayPeerResponse::CapabilityRuntimeToolHandled {
                result,
                skill_package,
            } => (result, skill_package),
            other => {
                return Err(DaemonError::LocalTransport {
                    operation: "forward leased capability runtime tool",
                    message: format!("unexpected forwarded capability response: {other:?}"),
                });
            }
        };
        if result.ok {
            if let Some(skill_package) = skill_package {
                let materialized_root = crate::skill::materialize_skill_package(
                    &crate::skill::remote_skill_materialization_base(&workspace_context.root)
                        .join(&remote_context.home_kernel_id),
                    &skill_package,
                )?;
                if let Some(skill) = result
                    .payload
                    .get_mut("skill")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    skill.insert(
                        "path".to_string(),
                        serde_json::Value::String(
                            materialized_root
                                .join("SKILL.md")
                                .to_string_lossy()
                                .to_string(),
                        ),
                    );
                    skill.insert(
                        "materialized_root".to_string(),
                        serde_json::Value::String(materialized_root.to_string_lossy().to_string()),
                    );
                    skill.insert(
                        "version_hash".to_string(),
                        serde_json::Value::String(skill_package.version_hash),
                    );
                    skill.insert(
                        "files".to_string(),
                        serde_json::Value::Array(
                            skill_package
                                .files
                                .into_iter()
                                .map(|file| serde_json::Value::String(file.path))
                                .collect(),
                        ),
                    );
                }
            }
        }
        Ok(Some(result))
    }

    pub(crate) async fn dispatch_forwarded_capability_runtime_tool_call(
        &self,
        context: crate::transport::relay_peer::RemoteManagedIoContext,
        tool_name: String,
        arguments: serde_json::Value,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        self.dispatch_capability_runtime_tool_call_for_agent(
            &context.home_session_id,
            &context.home_agent_id,
            &tool_name,
            arguments,
            true,
        )
        .await
    }

    pub(super) async fn dispatch_capability_runtime_tool_call(
        &self,
        provider_run: &crate::provider::RuntimeProviderRun,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let Some(agent_id) = provider_run.agent_instance_id().map(str::to_string) else {
            return Ok(crate::transport::runtime_tools::RuntimeToolResult {
                ok: false,
                payload: serde_json::json!({
                    "error": "capability tools require an agent-scoped provider run"
                }),
            });
        };
        let session_id = provider_run.session_id().to_string();
        let (result, _) = self
            .dispatch_capability_runtime_tool_call_for_agent(
                &session_id,
                &agent_id,
                tool_name,
                arguments,
                false,
            )
            .await?;
        Ok(result)
    }

    async fn dispatch_capability_runtime_tool_call_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
        include_skill_package: bool,
    ) -> Result<
        (
            crate::transport::runtime_tools::RuntimeToolResult,
            Option<crate::skill::ArrobaSkillPackage>,
        ),
        DaemonError,
    > {
        let session = self.owned.session_store.get_session(session_id)?;
        let agent = self.owned.agent_store.get_agent(agent_id)?;
        let mcp_registry = mcp_registry_for_workspace(session.workspace_id());
        let skill_registry = skill_registry_for_workspace(session.workspace_id());

        match tool_name {
            crate::transport::runtime_tools::LIST_EXTENSIONS_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::ListExtensionsArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_list_extensions",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let kind = args.kind.as_deref().unwrap_or("all");
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
                let mcps = if matches!(kind, "all" | "mcp") {
                    mcp_registry
                        .list()?
                        .into_iter()
                        .map(|mcp| {
                            let granted = agent.has_extension_grant(
                                crate::extension::ExtensionKind::Mcp,
                                &mcp.name,
                            );
                            serde_json::json!({
                                "kind": "mcp",
                                "name": mcp.name,
                                "enabled": mcp.enabled,
                                "required": mcp.required,
                                "granted": granted,
                                "effective_when_requested": "next_provider_launch"
                            })
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let skills = if matches!(kind, "all" | "skill") {
                    skill_registry
                        .list()?
                        .into_iter()
                        .map(|skill| {
                            let granted = agent.has_extension_grant(
                                crate::extension::ExtensionKind::Skill,
                                &skill.name,
                            );
                            serde_json::json!({
                                "kind": "skill",
                                "name": skill.name,
                                "description": skill.description,
                                "short_description": skill.short_description,
                                "granted": granted,
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
                            let granted = agent.has_extension_grant(
                                crate::extension::ExtensionKind::Script,
                                &script.name,
                            );
                            serde_json::json!({
                                "kind": "script",
                                "name": script.name,
                                "description": script.description,
                                "runtime": script.runtime,
                                "definition_hash": script.definition_hash,
                                "granted": granted,
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
                                    crate::connector::ConnectorSafety::parse(
                                        grant.max_safety.as_deref(),
                                    )
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
            crate::transport::runtime_tools::REQUEST_EXTENSION_TOOL => {
                let args = serde_json::from_value::<
                    crate::transport::runtime_tools::RequestExtensionArgs,
                >(arguments)
                .map_err(|error| DaemonError::LocalTransport {
                    operation: "runtime_tool_request_extension",
                    message: format!("invalid tool arguments: {error}"),
                })?;
                let mut skill_payload = serde_json::Value::Null;
                let mut skill_package = None;
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
                        if agent.remote_execution().is_some()
                            && !agent.has_extension_grant(
                                crate::extension::ExtensionKind::Mcp,
                                &args.name,
                            )
                        {
                            let mut checked = agent.clone();
                            checked.grant_mcp(args.name.clone());
                            self.ensure_remote_mcp_availability_for_agent(&checked)
                                .await?;
                        }
                        let granted_agent = self.owned.grant_agent_mcp(
                            agent.id(),
                            args.name.clone(),
                            agent.owner_user_id(),
                        )?;
                        self.append_agent_durable_event(
                            "agent.mcp_granted",
                            &granted_agent,
                            Some(&args.name),
                        )
                        .await?;
                        let (source_attachment_id, previous_prompt) = self
                            .owned
                            .session_store
                            .get_session(session_id)
                            .ok()
                            .and_then(|session| {
                                self.owned
                                    .prompt_state_owner
                                    .active_prompt_for_agent(&session, granted_agent.id())
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
                                    message: format!(
                                        "failed to read skill `{}` body: {error}",
                                        skill.name
                                    ),
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
                        let granted_agent = self.owned.grant_agent_skill(
                            agent.id(),
                            args.name.clone(),
                            agent.owner_user_id(),
                        )?;
                        self.append_agent_durable_event(
                            "agent.skill_granted",
                            &granted_agent,
                            Some(&args.name),
                        )
                        .await?;
                        (granted_agent, "now", false)
                    }
                    "script" => {
                        let environment = args.environment.clone().ok_or_else(|| {
                            DaemonError::LocalTransport {
                                operation: "runtime_tool_request_extension",
                                message: "script extension requests require environment"
                                    .to_string(),
                            }
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
                        let granted_agent = self.owned.grant_agent_extension(
                            agent.id(),
                            crate::extension::ExtensionGrant::script(
                                args.name.clone(),
                                environment,
                            ),
                            agent.owner_user_id(),
                        )?;
                        self.append_agent_durable_event(
                            "agent.extension_granted",
                            &granted_agent,
                            Some(&format!("script:{}", args.name)),
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
                        let max_safety =
                            crate::connector::ConnectorSafety::parse(args.allow.as_deref())?;
                        let granted_agent = self.owned.grant_agent_extension(
                            agent.id(),
                            crate::extension::ExtensionGrant::connector(
                                args.name.clone(),
                                args.credential.clone(),
                                max_safety.as_str(),
                            ),
                            agent.owner_user_id(),
                        )?;
                        self.append_agent_durable_event(
                            "agent.extension_granted",
                            &granted_agent,
                            Some(&format!("connector:{}", args.name)),
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
                        "now" => "The skill grant is persisted and the returned SKILL.md body can be followed immediately in this turn.",
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
            _ => Err(DaemonError::LocalTransport {
                operation: "dispatch_capability_runtime_tool_call",
                message: format!("unknown capability runtime tool `{tool_name}`"),
            }),
        }
    }
}
