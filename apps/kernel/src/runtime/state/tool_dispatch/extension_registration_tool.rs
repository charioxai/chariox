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
        Ok(registration_result(
            "mcp",
            &args.config.name,
            path,
            serde_json::json!({ "mcp": args.config }),
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
        Ok(registration_result(
            "skill",
            &skill.name,
            path,
            serde_json::json!({ "skill": skill }),
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
        Ok(registration_result(
            "environment",
            &args.config.name,
            path,
            serde_json::json!({ "environment": args.config }),
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
        Ok(registration_result(
            "script",
            &script.name,
            path,
            serde_json::json!({ "script": script }),
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
        Ok(registration_result(
            "connector",
            &connector.name,
            path,
            serde_json::json!({ "connector": connector }),
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
        Ok(registration_result(
            "connector_adapter",
            &adapter.name,
            path,
            serde_json::json!({ "adapter": adapter }),
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
        let authority = crate::session::effective_agent_user_authority(session, Some(agent));
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
            return Ok(None);
        }
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
}

fn invalid_registration_args(operation: &'static str) -> impl Fn(serde_json::Error) -> DaemonError {
    move |error| DaemonError::LocalTransport {
        operation,
        message: format!("invalid tool arguments: {error}"),
    }
}

fn registration_result(
    kind: &str,
    name: &str,
    path: std::path::PathBuf,
    extra: serde_json::Value,
) -> crate::transport::runtime_tools::RuntimeToolResult {
    let mut payload = serde_json::json!({
        "registered": true,
        "kind": kind,
        "name": name,
        "path": path,
        "granted": false,
        "next_action": "Call arroba.request_extension to grant this extension to the current agent before using it."
    });
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            payload.insert(key.clone(), value.clone());
        }
    }
    crate::transport::runtime_tools::RuntimeToolResult { ok: true, payload }
}
