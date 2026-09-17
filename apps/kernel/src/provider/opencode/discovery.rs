//! Ephemeral launch policy for project-environment discovery.

use std::collections::BTreeMap;

use crate::error::DaemonError;
use crate::provider::RuntimeProviderRun;

pub(crate) fn apply_opencode_discovery_environment(
    run: &RuntimeProviderRun,
    environment: &mut BTreeMap<String, String>,
) -> Result<(), DaemonError> {
    if run.adapter_key() != "opencode" || !run.read_only_discovery() {
        return Ok(());
    }
    let invalid_config = || DaemonError::ProviderProtocol {
        provider_run_id: run.id().to_string(),
        operation: "opencode_discovery_config",
        message: "OpenCode discovery requires an object-valued inline configuration".to_string(),
    };
    let mut config = match environment.get("OPENCODE_CONFIG_CONTENT") {
        Some(value) => {
            serde_json::from_str::<serde_json::Value>(value).map_err(|_| invalid_config())?
        }
        None => serde_json::json!({}),
    };
    let object = config.as_object_mut().ok_or_else(invalid_config)?;
    // Do not give the discovery child ordinary runtime/granted MCP credentials.
    // This changes only the spawn copy, so ordinary restoration retains them.
    object.insert("mcp".into(), serde_json::json!({}));
    let permissions = serde_json::json!({
        "*": "deny", "read": "allow", "glob": "allow", "grep": "allow", "list": "allow"
    });
    object.insert("permission".into(), permissions.clone());
    // Agent policy overrides global policy in OpenCode. The utility uses plan.
    let agents = object
        .entry("agent")
        .or_insert_with(|| serde_json::json!({}));
    let agents = agents.as_object_mut().ok_or_else(invalid_config)?;
    let plan = agents
        .entry("plan")
        .or_insert_with(|| serde_json::json!({}));
    plan.as_object_mut()
        .ok_or_else(invalid_config)?
        .insert("permission".into(), permissions);
    environment.insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
    environment.insert("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "true".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AgentEndpointMode, LaunchProviderRequest, ProviderLaunchResult};

    fn run(discovery: bool) -> RuntimeProviderRun {
        let request =
            LaunchProviderRequest::new("session", "opencode", "opencode", "default", "model");
        let mut run = RuntimeProviderRun::new(
            "run",
            &request,
            ProviderLaunchResult {
                endpoint_mode: AgentEndpointMode::Managed,
                process_label: "discovery-policy-test".into(),
                pty_target: None,
                pty_program: None,
                pty_args: Vec::new(),
                pty_env: BTreeMap::new(),
                pty_env_remove: Vec::new(),
                working_directory: None,
                structured_endpoint: None,
            },
        );
        run.set_read_only_discovery(discovery);
        run
    }

    #[test]
    fn discovery_removes_inline_mcp_and_overrides_plan_permissions() {
        let original = BTreeMap::from([(
            "OPENCODE_CONFIG_CONTENT".into(),
            serde_json::json!({
                "mcp": {"mutating": {"enabled": true}},
                "permission": "allow",
                "agent": {"plan": {"permission": "allow", "temperature": 0.2}},
                "model": "fixture/model"
            })
            .to_string(),
        )]);
        let mut child = original.clone();
        apply_opencode_discovery_environment(&run(true), &mut child).unwrap();
        let config: serde_json::Value =
            serde_json::from_str(&child["OPENCODE_CONFIG_CONTENT"]).unwrap();
        assert_eq!(config["mcp"], serde_json::json!({}));
        assert_eq!(config["permission"]["*"], "deny");
        assert_eq!(config["agent"]["plan"]["permission"], config["permission"]);
        assert_eq!(config["agent"]["plan"]["temperature"], 0.2);
        assert_eq!(config["model"], "fixture/model");
        assert_eq!(child["OPENCODE_DISABLE_PROJECT_CONFIG"], "true");
        assert_ne!(child, original);
    }

    #[test]
    fn ordinary_spawn_keeps_configuration_unchanged() {
        let mut environment =
            BTreeMap::from([("OPENCODE_CONFIG_CONTENT".into(), "original".into())]);
        let original = environment.clone();
        apply_opencode_discovery_environment(&run(false), &mut environment).unwrap();
        assert_eq!(environment, original);
    }

    #[test]
    fn discovery_rejects_invalid_config_without_mutating_environment() {
        for content in [
            "invalid",
            "[]",
            "null",
            r#"{"agent":false}"#,
            r#"{"agent":{"plan":false}}"#,
        ] {
            let mut environment =
                BTreeMap::from([("OPENCODE_CONFIG_CONTENT".into(), content.into())]);
            let original = environment.clone();
            assert!(apply_opencode_discovery_environment(&run(true), &mut environment).is_err());
            assert_eq!(environment, original);
        }
    }
}
