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
