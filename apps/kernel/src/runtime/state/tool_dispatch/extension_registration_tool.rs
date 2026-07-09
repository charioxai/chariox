use std::path::Path;

use crate::error::DaemonError;
use crate::runtime::state::KernelRuntimeState;

use super::capability_registry::{
    connector_adapter_registry, connector_registry, global_environment_registry,
    global_mcp_registry, global_script_registry, global_skill_registry, resolve_registration_path,
};

impl KernelRuntimeState {
    pub(super) async fn handle_register_mcp_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args =
            serde_json::from_value::<crate::transport::runtime_tools::RegisterMcpArgs>(arguments)
                .map_err(invalid_registration_args("runtime_tool_register_mcp"))?;
        if let Some(result) = self
            .maybe_gate_extension_registration(
                session,
                agent,
                "mcp",
                Some(args.config.name.as_str()),
                None,
            )
            .await?
        {
            return Ok(result);
        }
        let registry = global_mcp_registry()?;
        let path = registry.install(&args.config)?;
        self.append_extension_registration_audit_event(
            "extension.registration.created",
            session,
            agent,
            "mcp",
            Some(&args.config.name),
            None,
            Some(&path),
            Some(serde_json::json!({
                "transport_type": mcp_transport_kind(&args.config.transport),
                "enabled": args.config.enabled,
                "required": args.config.required,
            })),
        )?;
        let grant = self
            .grant_registered_extension_if_requested(
                session,
                agent,
                args.grant_to_current_agent.unwrap_or(false),
                "mcp",
                &args.config.name,
                None,
                None,
                None,
            )
            .await?;
        Ok(registration_result(
            "mcp",
            &args.config.name,
            path,
            serde_json::json!({ "mcp": args.config }),
            grant,
        ))
    }

    pub(super) async fn handle_register_skill_path_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args =
            serde_json::from_value::<crate::transport::runtime_tools::RegisterSkillPathArgs>(
                arguments,
            )
            .map_err(invalid_registration_args(
                "runtime_tool_register_skill_path",
            ))?;
        let source_path = resolve_registration_path(session, &args.path);
        if let Some(result) = self
            .maybe_gate_extension_registration(
                session,
                agent,
                "skill",
                None,
                Some(source_path.as_path()),
            )
            .await?
        {
            return Ok(result);
        }
        let registry = global_skill_registry()?;
        let (skill, path) = registry.upsert_from_path(&source_path)?;
        self.append_extension_registration_audit_event(
            "extension.registration.created",
            session,
            agent,
            "skill",
            Some(&skill.name),
            Some(&source_path),
            Some(&path),
            Some(serde_json::json!({
                "description": skill.description.as_str(),
                "short_description": skill.short_description.as_deref(),
            })),
        )?;
        let grant = self
            .grant_registered_extension_if_requested(
                session,
                agent,
                args.grant_to_current_agent.unwrap_or(false),
                "skill",
                &skill.name,
                None,
                None,
                None,
            )
            .await?;
        Ok(registration_result(
            "skill",
            &skill.name,
            path,
            serde_json::json!({ "skill": skill }),
            grant,
        ))
    }

    pub(super) async fn handle_register_environment_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args =
            serde_json::from_value::<crate::transport::runtime_tools::RegisterEnvironmentArgs>(
                arguments,
            )
            .map_err(invalid_registration_args(
                "runtime_tool_register_environment",
            ))?;
        if let Some(result) = self
            .maybe_gate_extension_registration(
                session,
                agent,
                "environment",
                Some(args.config.name.as_str()),
                None,
            )
            .await?
        {
            return Ok(result);
        }
        let registry = global_environment_registry()?;
        let path = registry.install(&args.config)?;
        self.append_extension_registration_audit_event(
            "extension.registration.created",
            session,
            agent,
            "environment",
            Some(&args.config.name),
            None,
            Some(&path),
            Some(serde_json::json!({
                "runtime": environment_runtime_kind(&args.config.runtime),
            })),
        )?;
        Ok(registration_result(
            "environment",
            &args.config.name,
            path,
            serde_json::json!({ "environment": args.config }),
            None,
        ))
    }

    pub(super) async fn handle_register_script_path_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args =
            serde_json::from_value::<crate::transport::runtime_tools::RegisterScriptPathArgs>(
                arguments,
            )
            .map_err(invalid_registration_args(
                "runtime_tool_register_script_path",
            ))?;
        let source_path = resolve_registration_path(session, &args.path);
        if let Some(result) = self
            .maybe_gate_extension_registration(
                session,
                agent,
                "script",
                args.name.as_deref(),
                Some(source_path.as_path()),
            )
            .await?
        {
            return Ok(result);
        }
        let env_registry = global_environment_registry()?;
        let env =
            env_registry
                .get(&args.environment)?
                .ok_or_else(|| DaemonError::LocalTransport {
                    operation: "runtime_tool_register_script_path",
                    message: format!("environment `{}` is not registered", args.environment),
                })?;
        let registry = global_script_registry()?;
        let (script, path) = registry.install(&source_path, args.name.as_deref(), &env)?;
        self.append_extension_registration_audit_event(
            "extension.registration.created",
            session,
            agent,
            "script",
            Some(&script.name),
            Some(&source_path),
            Some(&path),
            Some(serde_json::json!({
                "runtime": script_runtime_kind(&script.runtime),
                "definition_hash": script.definition_hash.as_str(),
            })),
        )?;
        let grant = self
            .grant_registered_extension_if_requested(
                session,
                agent,
                args.grant_to_current_agent.unwrap_or(false),
                "script",
                &script.name,
                Some(args.environment.as_str()),
                None,
                None,
            )
            .await?;
        Ok(registration_result(
            "script",
            &script.name,
            path,
            serde_json::json!({ "script": script }),
            grant,
        ))
    }

    pub(super) async fn handle_register_connector_path_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::RegisterConnectorPathArgs,
        >(arguments)
        .map_err(invalid_registration_args(
            "runtime_tool_register_connector_path",
        ))?;
        let source_path = resolve_registration_path(session, &args.path);
        if let Some(result) = self
            .maybe_gate_extension_registration(
                session,
                agent,
                "connector",
                None,
                Some(source_path.as_path()),
            )
            .await?
        {
            return Ok(result);
        }
        let registry = connector_registry()?;
        let adapters = connector_adapter_registry()?;
        let (connector, path) = registry.install_from_file(&source_path, &adapters)?;
        self.append_extension_registration_audit_event(
            "extension.registration.created",
            session,
            agent,
            "connector",
            Some(&connector.name),
            Some(&source_path),
            Some(&path),
            Some(serde_json::json!({
                "adapter": connector.adapter.as_str(),
                "operation_count": connector.operations.len(),
                "credential_required": connector
                    .credential
                    .as_ref()
                    .map(|credential| credential.required),
            })),
        )?;
        let grant = self
            .grant_registered_extension_if_requested(
                session,
                agent,
                args.grant_to_current_agent.unwrap_or(false),
                "connector",
                &connector.name,
                None,
                args.credential.as_deref(),
                args.allow.as_deref(),
            )
            .await?;
        Ok(registration_result(
            "connector",
            &connector.name,
            path,
            serde_json::json!({ "connector": connector }),
            grant,
        ))
    }

    pub(super) async fn handle_register_connector_adapter_path_runtime_tool(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        arguments: serde_json::Value,
    ) -> Result<crate::transport::runtime_tools::RuntimeToolResult, DaemonError> {
        let args = serde_json::from_value::<
            crate::transport::runtime_tools::RegisterConnectorAdapterPathArgs,
        >(arguments)
        .map_err(invalid_registration_args(
            "runtime_tool_register_connector_adapter_path",
        ))?;
        let source_path = resolve_registration_path(session, &args.path);
        if let Some(result) = self
            .maybe_gate_extension_registration(
                session,
                agent,
                "connector_adapter",
                None,
                Some(source_path.as_path()),
            )
            .await?
        {
            return Ok(result);
        }
        let registry = connector_adapter_registry()?;
        let (adapter, path) = registry.install_from_file(&source_path)?;
        self.append_extension_registration_audit_event(
            "extension.registration.created",
            session,
            agent,
            "connector_adapter",
            Some(&adapter.name),
            Some(&source_path),
            Some(&path),
            Some(serde_json::json!({
                "command": adapter.command.display().to_string(),
                "arg_count": adapter.args.len(),
            })),
        )?;
        Ok(registration_result(
            "connector_adapter",
            &adapter.name,
            path,
            serde_json::json!({ "adapter": adapter }),
            None,
        ))
    }

    async fn maybe_gate_extension_registration(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        kind: &str,
        name: Option<&str>,
        source_path: Option<&Path>,
    ) -> Result<Option<crate::transport::runtime_tools::RuntimeToolResult>, DaemonError> {
        let authority =
            crate::session::effective_agent_extension_registration_authority(session, Some(agent));
        if !authority.requires_approval() {
            return Ok(None);
        }
        let title = "Arroba extension registration approval".to_string();
        let subject = name
            .map(|name| format!("global Arroba {kind} `{name}`"))
            .unwrap_or_else(|| format!("global Arroba {kind}"));
        let source = source_path
            .map(|path| format!(" Source: `{}`.", path.display()))
            .unwrap_or_default();
        let interaction = crate::session::RuntimeInteraction::new(
            format!(
                "extension-registration-permission-{}-{}",
                agent.id(),
                crate::session::unix_epoch_ms()
            ),
            agent.id(),
            crate::session::RuntimeInteractionKind::Permission,
            crate::session::RuntimeInteractionLevel::Warning,
            Some(title),
            format!(
                "Allow agent `{}` to register {subject}?{source}",
                agent.agent_ref()
            ),
            vec![
                crate::session::RuntimeInteractionChoice::new(
                    "allow",
                    "Allow",
                    "allow",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Primary),
                ),
                crate::session::RuntimeInteractionChoice::new(
                    "deny",
                    "Deny",
                    "deny",
                    Some(crate::session::RuntimeInteractionChoiceStyle::Danger),
                ),
            ],
            None,
            None,
            None,
        );
        let interaction_id = interaction.id().to_string();
        let resolution = self
            .create_runtime_interaction(session.id(), interaction)
            .await?
            .await
            .map_err(|error| DaemonError::LocalTransport {
                operation: "extension_registration_permission",
                message: format!(
                    "extension registration approval dropped before resolution: {error}"
                ),
            })?;
        if resolution.choice_id.as_deref() == Some("allow") {
            self.append_extension_registration_audit_event(
                "extension.registration.approved",
                session,
                agent,
                kind,
                name,
                source_path,
                None,
                Some(serde_json::json!({ "interaction_id": interaction_id })),
            )?;
            return Ok(None);
        }
        self.append_extension_registration_audit_event(
            "extension.registration.denied",
            session,
            agent,
            kind,
            name,
            source_path,
            None,
            Some(serde_json::json!({ "interaction_id": interaction_id })),
        )?;
        Ok(Some(crate::transport::runtime_tools::RuntimeToolResult {
            ok: false,
            payload: serde_json::json!({
                "registered": false,
                "interaction_id": interaction_id,
                "kind": kind,
                "name": name,
                "source_path": source_path.map(|path| path.display().to_string()),
                "reason": {
                    "kind": "permission_denied",
                    "message": "The extension registration operation was not approved."
                }
            }),
        }))
    }

    fn append_extension_registration_audit_event(
        &self,
        event_kind: &'static str,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        extension_kind: &str,
        name: Option<&str>,
        source_path: Option<&Path>,
        registry_path: Option<&Path>,
        detail: Option<serde_json::Value>,
    ) -> Result<(), DaemonError> {
        let mut payload = serde_json::Map::new();
        payload.insert("session_id".to_string(), serde_json::json!(session.id()));
        payload.insert(
            "session_owner_user_id".to_string(),
            serde_json::json!(session.owner_user_id()),
        );
        payload.insert("agent_id".to_string(), serde_json::json!(agent.id()));
        payload.insert(
            "agent_ref".to_string(),
            serde_json::json!(agent.agent_ref()),
        );
        payload.insert(
            "agent_owner_user_id".to_string(),
            serde_json::json!(agent.owner_user_id()),
        );
        payload.insert("kind".to_string(), serde_json::json!(extension_kind));
        payload.insert("name".to_string(), serde_json::json!(name));
        payload.insert(
            "source_path".to_string(),
            serde_json::json!(source_path.map(|path| path.display().to_string())),
        );
        payload.insert(
            "registry_path".to_string(),
            serde_json::json!(registry_path.map(|path| path.display().to_string())),
        );
        if let Some(detail) = detail {
            payload.insert("detail".to_string(), detail);
        }
        self.owned.durable_state_store.append_event(
            event_kind,
            Some(agent.id().to_string()),
            serde_json::Value::Object(payload),
        )?;
        Ok(())
    }

    async fn grant_registered_extension_if_requested(
        &self,
        session: &crate::session::RuntimeSession,
        agent: &crate::agent::AgentInstance,
        requested: bool,
        kind: &str,
        name: &str,
        environment: Option<&str>,
        credential: Option<&str>,
        allow: Option<&str>,
    ) -> Result<Option<RegistrationGrantResult>, DaemonError> {
        if !requested {
            return Ok(None);
        }
        let (granted_agent, effective, requires_provider_restart) = match kind {
            "mcp" => {
                let granted_agent = self
                    .grant_agent_mcp(agent.id(), name.to_string(), agent.owner_user_id())
                    .await?;
                self.remember_registration_mcp_continuation(session.id(), &granted_agent, name);
                (granted_agent, "after_provider_reload", true)
            }
            "skill" => {
                let granted_agent = self
                    .grant_agent_skill(agent.id(), name.to_string(), agent.owner_user_id())
                    .await?;
                (granted_agent, "now", false)
            }
            "script" => {
                let environment = environment.ok_or_else(|| DaemonError::LocalTransport {
                    operation: "runtime_tool_register_script_path",
                    message: "script registration grants require an environment".to_string(),
                })?;
                let granted_agent = self
                    .grant_agent_extension(
                        agent.id(),
                        crate::extension::ExtensionGrant::script(name.to_string(), environment),
                        agent.owner_user_id(),
                    )
                    .await?;
                (granted_agent, "now", false)
            }
            "connector" => {
                if let Some(credential) = credential {
                    crate::runtime::capability_registry::ensure_credential_exists(credential)?;
                }
                let max_safety = crate::connector::ConnectorSafety::parse(allow)?;
                let granted_agent = self
                    .grant_agent_extension(
                        agent.id(),
                        crate::extension::ExtensionGrant::connector(
                            name.to_string(),
                            credential.map(str::to_string),
                            max_safety.as_str(),
                        ),
                        agent.owner_user_id(),
                    )
                    .await?;
                (granted_agent, "now", false)
            }
            _ => {
                return Err(DaemonError::LocalTransport {
                    operation: "runtime_tool_register_extension",
                    message: format!("registration grant is not supported for `{kind}`"),
                });
            }
        };
        Ok(Some(RegistrationGrantResult {
            agent_ref: granted_agent.agent_ref().to_string(),
            effective,
            requires_provider_restart,
        }))
    }

    fn remember_registration_mcp_continuation(
        &self,
        session_id: &str,
        granted_agent: &crate::agent::AgentInstance,
        mcp_name: &str,
    ) {
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
            mcp_name,
            &previous_prompt,
        );
    }
}

fn invalid_registration_args(operation: &'static str) -> impl Fn(serde_json::Error) -> DaemonError {
    move |error| DaemonError::LocalTransport {
        operation,
        message: format!("invalid tool arguments: {error}"),
    }
}

fn mcp_transport_kind(transport: &crate::mcp::ArrobaMcpTransportConfig) -> &'static str {
    match transport {
        crate::mcp::ArrobaMcpTransportConfig::Stdio { .. } => "stdio",
        crate::mcp::ArrobaMcpTransportConfig::StreamableHttp { .. } => "streamable_http",
    }
}

fn environment_runtime_kind(runtime: &crate::script::ArrobaEnvironmentRuntime) -> &'static str {
    match runtime {
        crate::script::ArrobaEnvironmentRuntime::Python { .. } => "python",
        crate::script::ArrobaEnvironmentRuntime::Node { .. } => "node",
    }
}

fn script_runtime_kind(runtime: &crate::script::ArrobaScriptRuntime) -> &'static str {
    match runtime {
        crate::script::ArrobaScriptRuntime::Python => "python",
        crate::script::ArrobaScriptRuntime::TypeScript => "typescript",
    }
}

struct RegistrationGrantResult {
    agent_ref: String,
    effective: &'static str,
    requires_provider_restart: bool,
}

fn registration_result(
    kind: &str,
    name: &str,
    path: std::path::PathBuf,
    extra: serde_json::Value,
    grant: Option<RegistrationGrantResult>,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let granted = grant.is_some();
    let mut payload = serde_json::json!({
        "registered": true,
        "kind": kind,
        "name": name,
        "path": path,
        "granted": granted,
    });
    if let Some(grant) = grant {
        payload["grant"] = serde_json::json!({
            "agent_ref": grant.agent_ref,
            "effective": grant.effective,
            "requires_provider_restart": grant.requires_provider_restart,
        });
    } else {
        payload["next_action"] = serde_json::json!(
            "Call arroba.request_extension to grant this extension to the current agent before using it."
        );
    }
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            payload.insert(key.clone(), value.clone());
        }
    }
    crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
}
